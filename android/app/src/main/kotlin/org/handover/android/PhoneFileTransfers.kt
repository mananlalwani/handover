package org.handover.android

import android.net.Uri
import android.os.CancellationSignal
import android.util.Base64
import org.json.JSONArray
import org.json.JSONObject
import java.io.*
import java.net.SocketTimeoutException
import java.nio.channels.FileChannel
import java.security.MessageDigest
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit
import javax.net.ssl.SSLSocket

/** Negotiated, framed phone downloads and stable directory snapshots. */
internal class PhoneFileTransfers(private val transport: NativeTransport) : Closeable {
    private data class Session(val socket: SSLSocket, val output: BufferedOutputStream, val fingerprint: String?, val access: PhoneFilesAccess)
    private data class Handle(val id: String, val path: String, val name: String, val size: Long,
        val version: String, val stream: InputStream, val channel: FileChannel?, val temp: File?, val session: Session, val reserved: Long,
        @Volatile var lastUse: Long = System.nanoTime(), val lock: Any = Any(), var reads: Int = 0)
    private data class Snapshot(val id: String, val path: String, val entries: List<PhoneFileEntry>, val session: Session,
        val bytes: Long, @Volatile var lastUse: Long = System.nanoTime())

    private data class Preparation(val session: Session, val cancellation: CancellationSignal,
        val source: java.util.concurrent.atomic.AtomicReference<Closeable?> = java.util.concurrent.atomic.AtomicReference(),
        val cancelled: java.util.concurrent.atomic.AtomicBoolean = java.util.concurrent.atomic.AtomicBoolean(),
        var timeout: java.util.concurrent.ScheduledFuture<*>? = null, var reserved: Long = 0, var stagedHandle: Boolean = false)
    private val preparing = ConcurrentHashMap<String, Preparation>()
    private val expiry = ConcurrentHashMap<String, java.util.concurrent.ScheduledFuture<*>>()
    private val resourceSlots=java.util.concurrent.Semaphore(8)
    private val cleanup=java.util.concurrent.ThreadPoolExecutor(2,2,0,TimeUnit.MILLISECONDS,
        java.util.concurrent.ArrayBlockingQueue(8),{ task -> Thread(task,"handover-file-close").apply{isDaemon=true} },java.util.concurrent.ThreadPoolExecutor.AbortPolicy())
    private val stageDirectory=File(transport.context.cacheDir,"phone-v2-transfers").apply {
        mkdirs()
        if (processCleanup.compareAndSet(false,true)) listFiles()?.filter{it.isFile&&it.name.startsWith("file-")&&it.name.endsWith(".part")}?.forEach{it.delete()}
    }
    private fun clean(task: () -> Unit) { try { cleanup.execute(task) } catch (_:java.util.concurrent.RejectedExecutionException) { task() } }
    private val handles = ConcurrentHashMap<String, Handle>()
    private val snapshots = LinkedHashMap<String, Snapshot>()
    private var stagedBytes = 0L
    private var stagedHandles = 0
    private var snapshotBytes = 0L
    private val closed = java.util.concurrent.atomic.AtomicBoolean()

    fun handle(message: JSONObject) {
        if (closed.get() || message.optInt("protocol", -1) != 1) return
        expireHandles()
        when (message.optString("type")) {
            "file_open_v2" -> open(message)
            "file_read_v2" -> read(message)
            "file_close_v2" -> closeHandle(message.optString("transfer_id"))
            "filesystem_list" -> if (message.optBoolean("paged", false)) listPage(message)
        }
    }

    private fun capture(): Session? {
        val s = transport.socket ?: return null
        val o = transport.output ?: return null
        if (s.isClosed) return null
        return Session(s, o, transport.serverFingerprint, PhoneFilesAccess(transport))
    }
    private fun active(session: Session, deadline: Long) {
        if (System.nanoTime() > deadline) throw SocketTimeoutException()
        if (closed.get() || transport.socket !== session.socket || transport.output !== session.output || transport.serverFingerprint != session.fingerprint) throw IOException("session ended")
        session.access.check()
    }
    private fun send(session: Session, value: JSONObject, check: Boolean = false) {
        transport.enqueue {
            synchronized(transport.outputLock) {
                if (transport.socket === session.socket && transport.output === session.output && transport.serverFingerprint == session.fingerprint) {
                    if (check || value.optString("type") in setOf("file_preparing_v2", "file_info_v2", "file_chunk_v2", "filesystem_entries")) {
                        runCatching { session.access.check() }.getOrElse {
                            if (value.optString("type").startsWith("file_")) {
                                val id = value.optString("transfer_id")
                                closeHandle(id)
                                transport.write(session.output, JSONObject().put("type", "file_error_v2").put("protocol",1)
                                    .put("transfer_id",id).put("reason","permission_required"))
                            } else transport.write(session.output, listingFailure(value.optString("request_id"),value.optString("path"),"permission_required"))
                            return@synchronized
                        }
                    }
                    if (value.optString("type") == "file_preparing_v2" && !preparing.containsKey(value.optString("transfer_id")) && !handles.containsKey(value.optString("transfer_id"))) return@synchronized
                    if (value.optString("type") in setOf("file_info_v2", "file_chunk_v2") && !handles.containsKey(value.optString("transfer_id"))) return@synchronized
                    runCatching { transport.write(session.output, value) }.onFailure { runCatching { session.socket.close() } }
                }
            }
        }
    }
    private fun fail(session: Session, id: String, reason: String) = send(session, JSONObject().put("type", "file_error_v2")
        .put("protocol", 1).put("transfer_id", id).put("reason", reason))
    private fun open(m: JSONObject) {
        val id = m.optString("transfer_id")
        val session = capture() ?: return
        if (!id.matches(Regex("[0-9a-f]{32}"))) return
        val path = m.optString("path")
        val expected = m.optString("expected_version").takeIf { !m.isNull("expected_version") }
        if (expected != null && !expected.matches(Regex("[0-9a-f]{64}"))) { fail(session,id,"invalid_resource"); return }
        val parts = runCatching { safeTreePathParts(path).also { require(it.isNotEmpty()) } }.getOrElse { fail(session,id,"invalid_resource"); return }
        if (!resourceSlots.tryAcquire()) { fail(session,id,"unavailable");return }
        val prep = Preparation(session, CancellationSignal())
        synchronized(preparing) {
            if (handles.size + preparing.size >= 8 || handles.containsKey(id) || preparing.putIfAbsent(id, prep) != null) {
                resourceSlots.release();fail(session,id,"unavailable"); return
            }
        }
        val deadline = System.nanoTime() + TimeUnit.MINUTES.toNanos(15)
        val cancellation = prep.cancellation
        prep.timeout = transport.fileRequestTimeout.schedule({ closeHandle(id) },15,TimeUnit.MINUTES)
        fun preparingActive() {
            if (prep.cancelled.get() || preparing[id] !== prep) throw IOException("preparation cancelled")
            active(session, deadline)
        }
        try {
            transport.fileRequestWorkers.execute {
                var source: PhoneFileSource? = null
                var temp: File? = null
                var handedOff = false
                try {
                    preparingActive()
                    source = session.access.open(path, cancellation) { preparingActive() }
                    prep.source.set(source)
                    if (source.name != parts.last()) throw IllegalArgumentException()
                    val initialSize = source.size
                    if (initialSize != null && initialSize > MAX_BYTES) throw TooLarge()
                    val fis = source.input as? FileInputStream
                    val channel = fis?.channel?.takeIf { runCatching { it.position(0) }.isSuccess }
                    val digest = MessageDigest.getInstance("SHA-256")
                    var total = 0L
                    var lastProgress=0L
                    fun progress() {
                        val now=System.nanoTime()
                        if (lastProgress==0L || now-lastProgress>=TimeUnit.SECONDS.toNanos(1)) {
                            lastProgress=now
                            send(session,JSONObject().put("type","file_preparing_v2").put("protocol",1).put("transfer_id",id)
                                .put("processed_bytes",total).apply {initialSize?.let{put("total_bytes",it)}})
                        }
                    }
                    progress()
                    val buffer = ByteArray(32 * 1024)
                    if (channel == null) {
                        synchronized(this) {
                            if (stagedHandles >= 8) throw IOException("staging handles full")
                            if (initialSize != null && initialSize > 0 && transport.context.cacheDir.usableSpace < initialSize) throw IOException("storage")
                            stagedHandles++
                            prep.stagedHandle = true
                            temp = File.createTempFile("file-", ".part", stageDirectory)
                        }
                    } else channel.position(0)
                    val stagedOut = temp?.outputStream()
                    try {
                        while (true) {
                            preparingActive()
                            val n = source.input.read(buffer)
                            preparingActive()
                            if (n < 0) break
                            if (n == 0) continue
                            if (n.toLong() > MAX_BYTES - total) throw TooLarge()
                            synchronized(this) {
                                if (temp != null && n > transport.context.cacheDir.usableSpace) throw IOException("storage")
                                if (temp != null && stagedBytes + n > MAX_STAGED) throw IOException("storage")
                                if (temp != null) { stagedBytes += n; prep.reserved += n }
                            }
                            digest.update(buffer,0,n); stagedOut?.write(buffer,0,n); total += n; progress()
                        }
                    } finally { stagedOut?.close() }
                    if (initialSize != null && total != initialSize) throw Changed()
                    val version = digest.digest().joinToString("") { "%02x".format(it) }
                    if (expected != null && expected != version) throw Changed()
                    preparingActive()
                    val fileName = source.name
                    val readyStream: InputStream
                    val readyChannel: FileChannel?
                    if (temp != null) { source.close(); source = null; readyStream = FileInputStream(temp!!); readyChannel = (readyStream as FileInputStream).channel }
                    else { channel!!.position(0); readyStream = source.input; readyChannel = channel; source = null }
                    val handle = Handle(id,path,fileName,total,version,readyStream,readyChannel,temp,session,prep.reserved)
                    preparingActive()
                    synchronized(preparing) {
                        preparingActive()
                        val prior = handles.putIfAbsent(id,handle)
                        if (prior != null) { runCatching { readyStream.close() }; throw IOException("duplicate transfer") }
                        preparing.remove(id,prep)
                        handedOff = true
                    }
                    prep.timeout?.cancel(false)
                    prep.source.set(null)
                    scheduleExpiry(handle)
                    send(session, JSONObject().put("type","file_info_v2").put("protocol",1).put("transfer_id",id)
                        .put("name",handle.name).put("size",total).put("version",version))
                } catch (e: Exception) {
                    when (e) {
                        is TooLarge -> fail(session,id,"size_limit")
                        is Changed -> fail(session,id,"source_changed")
                        is SecurityException -> fail(session,id,"permission_required")
                        is IllegalArgumentException -> fail(session,id,"invalid_resource")
                        is SocketTimeoutException -> fail(session,id,"interrupted")
                        else -> fail(session,id, if (e.message == "storage") "storage" else "unavailable")
                    }
                } finally { preparing.remove(id,prep); prep.timeout?.cancel(false); prep.source.set(null); runCatching { source?.close() }; if (!handedOff) { synchronized(this) { stagedBytes = (stagedBytes - prep.reserved).coerceAtLeast(0); if (prep.stagedHandle) stagedHandles-- }; temp?.delete();resourceSlots.release() } }
            }
        } catch (_: java.util.concurrent.RejectedExecutionException) { preparing.remove(id,prep); prep.timeout?.cancel(false);resourceSlots.release();fail(session,id,"unavailable") }
    }
    private fun read(m: JSONObject) {
        val id=m.optString("transfer_id"); val h=handles[id] ?: return
        val off=integer(m,"offset") ?: run { fail(h.session,id,"invalid_resource"); return }
        val amount=integer(m,"size") ?: run { fail(h.session,id,"invalid_resource"); return }
        if (amount !in 1..32768 || off > h.size || amount > h.size-off) { fail(h.session,id,"invalid_resource"); return }
        synchronized(h.lock) { if (h.reads >= 4) { fail(h.session,id,"unavailable"); return }; h.reads++ }
        val deadline=System.nanoTime()+TimeUnit.SECONDS.toNanos(120)
        h.lastUse=System.nanoTime()
        try { transport.fileRequestWorkers.execute {
            try {
                active(h.session,deadline)
                val data=ByteArray(amount.toInt())
                synchronized(h.lock) {
                    if (handles[id] !== h) return@execute
                    h.channel?.position(off)
                    var done=0
                    while (done<data.size) {
                        active(h.session,deadline)
                        val n=h.stream.read(data,done,data.size-done); if(n<0) throw Changed(); if(n>0) done+=n
                    }
                    h.lastUse=System.nanoTime()
                }
                if (handles[id] === h) send(h.session,JSONObject().put("type","file_chunk_v2").put("protocol",1)
                    .put("transfer_id",id).put("offset",off).put("data",Base64.encodeToString(data,Base64.NO_WRAP)))
            } catch (e: Exception) { if(handles[id]===h) fail(h.session,id,if(e is SecurityException) "permission_required" else if(e is Changed) "source_changed" else "interrupted") }
            finally { synchronized(h.lock) { h.reads-- } }
        } } catch (_: java.util.concurrent.RejectedExecutionException) { synchronized(h.lock){h.reads--}; fail(h.session,id,"unavailable") }
    }
    private fun integer(m: JSONObject,key:String):Long? = when(val value=m.opt(key)) {
        is Int -> value.toLong().takeIf { it >= 0 }
        is Long -> value.takeIf { it >= 0 }
        else -> null
    }
    private fun scheduleExpiry(handle: Handle) {
        if (closed.get() || handles[handle.id] !== handle) return
        val remaining=(TimeUnit.SECONDS.toNanos(120)-(System.nanoTime()-handle.lastUse)).coerceAtLeast(1)
        val future=transport.fileRequestTimeout.schedule({
            if (handles[handle.id] === handle) {
                if (System.nanoTime()-handle.lastUse >= TimeUnit.SECONDS.toNanos(120)) closeHandle(handle.id)
                else scheduleExpiry(handle)
            }
        },remaining,TimeUnit.NANOSECONDS)
        expiry.put(handle.id,future)?.cancel(false)
        if (handles[handle.id] !== handle && expiry.remove(handle.id,future)) future.cancel(false)
    }
    private fun closeHandle(id:String) {
        expiry.remove(id)?.cancel(false)
        synchronized(preparing) { preparing.remove(id) }?.let { prep ->
            prep.cancelled.set(true);prep.timeout?.cancel(false)
            clean { prep.cancellation.cancel();runCatching {prep.source.getAndSet(null)?.close()} }
        }
        handles.remove(id)?.let { handle -> clean { dispose(handle) } }
    }
    private fun dispose(handle:Handle) {
        try {
            runCatching {handle.stream.close()}
            handle.temp?.let { synchronized(this){stagedBytes=(stagedBytes-handle.reserved).coerceAtLeast(0);stagedHandles--};it.delete() }
        }finally{resourceSlots.release()}
    }

    private fun listPage(m:JSONObject) {
        val session=capture() ?: return; val id=m.optString("request_id"); val path=m.optString("path",".")
        if(!id.matches(Regex("[0-9a-f]{32}"))) return
        runCatching{safeTreePathParts(path)}.getOrElse{ send(session,listingFailure(id,path,"unavailable")); return }
        val cursor=m.optString("cursor").takeIf{!m.isNull("cursor")}
        try { transport.fileRequestWorkers.execute {
            try {
                val deadline=System.nanoTime()+TimeUnit.SECONDS.toNanos(30)
                active(session,deadline)
                val snapshot:Snapshot
                val offset:Int
                if (cursor==null) {
                    val cancellation=CancellationSignal()
                    val abort=Closeable { cancellation.cancel() }
                    transport.fileRequestAborts.add(abort)
                    val timer=transport.fileRequestTimeout.schedule({abort.close()},30,TimeUnit.SECONDS)
                    val entries=try { session.access.list(path,cancellation,{active(session,deadline)},50001) }
                        finally { timer.cancel(false); transport.fileRequestAborts.remove(abort) }
                    require(entries.size<=50000)
                    val sorted=entries.sortedWith(compareByDescending<PhoneFileEntry>{it.directory}.thenBy{it.name.lowercase(java.util.Locale.ROOT)}.thenBy{it.name})
                    val bytes=sorted.sumOf { entry -> entry.name.length.toLong()*2+96 }
                    require(bytes<=MAX_SNAPSHOT_BYTES)
                    synchronized(snapshots) {
                        expireSnapshots()
                        while (snapshots.size>=8 || snapshotBytes+bytes>MAX_SNAPSHOT_BYTES) {
                            val old=snapshots.values.minByOrNull{it.lastUse} ?: break
                            snapshots.remove(old.id); snapshotBytes-=old.bytes
                        }
                        val snap=Snapshot(UUID.randomUUID().toString(),path,sorted,session,bytes)
                        snapshots[snap.id]=snap;snapshotBytes+=bytes;snapshot=snap;offset=0
                    }
                }else{
                    synchronized(snapshots) {
                        expireSnapshots()
                        val split=cursor.split(':');require(split.size==2)
                        val snap=snapshots[split[0]] ?: throw IllegalArgumentException()
                        require(snap.path==path&&snap.session.socket===session.socket&&snap.session.fingerprint==session.fingerprint)
                        offset=split[1].toInt();require(offset>=0&&offset<snap.entries.size)
                        snap.lastUse=System.nanoTime();snapshot=snap
                    }
                }
                val pageDeadline=System.nanoTime()+TimeUnit.SECONDS.toNanos(30)
                active(snapshot.session,pageDeadline)
                val array=JSONArray(); var bytes=0; var next=offset
                while(next<snapshot.entries.size && array.length()<128) {
                    active(snapshot.session,pageDeadline); val item=snapshot.entries[next]
                    val entry=JSONObject().put("name",item.name).put("directory",item.directory); item.size?.let{if(!item.directory)entry.put("size",it)}
                    val sz=entry.toString().toByteArray(Charsets.UTF_8).size+1; if(array.length()>0 && bytes+sz>48*1024)break
                    require(sz<=48*1024); array.put(entry); bytes+=sz; next++
                }
                val reply=JSONObject().put("type","filesystem_entries").put("protocol",1).put("request_id",id).put("path",path).put("entries",array)
                if(next<snapshot.entries.size)reply.put("cursor_next",snapshot.id+":"+next) else synchronized(snapshots){if(snapshots.remove(snapshot.id)!=null)snapshotBytes-=snapshot.bytes}
                send(snapshot.session,reply,true)
            } catch(e:Exception){send(session,listingFailure(id,path,if(e is SecurityException)"permission_required" else "unavailable"),true)}
        } } catch(_:java.util.concurrent.RejectedExecutionException){send(session,listingFailure(id,path,"unavailable"))}
    }
    private fun listingFailure(id:String,path:String,reason:String)=JSONObject().put("type","filesystem_failure").put("protocol",1).put("request_id",id).put("path",path).put("reason",reason)
    private fun expireSnapshots(){val cutoff=System.nanoTime()-TimeUnit.MINUTES.toNanos(5); val expired=snapshots.values.filter{it.lastUse<cutoff}; expired.forEach{snapshots.remove(it.id);snapshotBytes-=it.bytes}}
    private fun expireHandles(){val cutoff=System.nanoTime()-TimeUnit.SECONDS.toNanos(120); handles.values.filter{it.lastUse<cutoff}.forEach{if(handles.remove(it.id,it))dispose(it)}}
    fun closeSession(socket:SSLSocket?) {
        preparing.entries.filter{socket==null || it.value.session.socket===socket}.forEach{closeHandle(it.key)}
        handles.values.filter{socket==null || it.session.socket===socket}.forEach{closeHandle(it.id)}
        synchronized(snapshots){val expired=snapshots.values.filter{socket==null || it.session.socket===socket}; expired.forEach{snapshots.remove(it.id);snapshotBytes-=it.bytes}}
    }
    override fun close(){if(!closed.compareAndSet(false,true))return; closeSession(null); synchronized(snapshots){snapshots.clear();snapshotBytes=0};cleanup.shutdown()}
    private class TooLarge:IOException()
    private class Changed:IOException()
    companion object { private val processCleanup=java.util.concurrent.atomic.AtomicBoolean();private const val MAX_BYTES=8L*1024*1024*1024; private const val MAX_STAGED=16L*1024*1024*1024; private const val MAX_SNAPSHOT_BYTES=8L*1024*1024 }
}
