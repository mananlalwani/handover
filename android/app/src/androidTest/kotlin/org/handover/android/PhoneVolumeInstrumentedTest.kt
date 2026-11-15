package org.handover.android

import android.content.Context
import android.media.AudioManager
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.BufferedOutputStream
import java.io.DataInputStream
import java.io.PipedInputStream
import java.io.PipedOutputStream
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLSocket

@RunWith(AndroidJUnit4::class)
class PhoneVolumeInstrumentedTest {
    @Test(timeout=60000) fun setterIsFollowedByReportedLevelsAndInvalidRequestsDoNotChangeVolume() {
        val context:Context=ApplicationProvider.getApplicationContext()
        val manager=context.getSystemService(AudioManager::class.java)
        org.junit.Assume.assumeFalse(manager.isVolumeFixed)
        val original=manager.getStreamVolume(AudioManager.STREAM_MUSIC)
        val maximum=manager.getStreamMaxVolume(AudioManager.STREAM_MUSIC)
        val minimum=if(android.os.Build.VERSION.SDK_INT>=28)manager.getStreamMinVolume(AudioManager.STREAM_MUSIC)else 0
        org.junit.Assume.assumeTrue(maximum>minimum)
        val desired=if(original==maximum)original-1 else original+1
        val pipe=PipedInputStream(65536);val output=PipedOutputStream(pipe);val input=DataInputStream(pipe)
        val transport=NativeTransport(context)
        fun frame():JSONObject {val size=input.readInt();require(size in 1..65536);val data=ByteArray(size);input.readFully(data);return JSONObject(String(data,Charsets.UTF_8))}
        try {
            transport.socket=SSLContext.getDefault().socketFactory.createSocket() as SSLSocket
            transport.output=BufferedOutputStream(output);transport.serverFingerprint="fixture-peer"
            transport.handlePhoneVolumeSet(JSONObject().put("request_id","0".repeat(31)+"1").put("kind","media").put("level",desired))
            val result=frame();assertEquals("volume_result",result.getString("type"));assertTrue(result.getBoolean("accepted"))
            val state=frame();assertEquals("volume_state",state.getString("type"))
            val streams=state.getJSONArray("streams")
            val media=(0 until streams.length()).map{streams.getJSONObject(it)}.single{it.getString("kind")=="media"}
            assertEquals(manager.getStreamVolume(AudioManager.STREAM_MUSIC),media.getInt("level"))
            assertEquals(desired,media.getInt("level"))
            transport.handlePhoneVolumeSet(JSONObject().put("request_id","0".repeat(31)+"2").put("kind","media").put("level",maximum+1))
            assertEquals("invalid_value",frame().getString("failure"));assertEquals("volume_state",frame().getString("type"))
            assertEquals(desired,manager.getStreamVolume(AudioManager.STREAM_MUSIC))
        }finally{manager.setStreamVolume(AudioManager.STREAM_MUSIC,original,0);transport.stop();pipe.close();output.close()}
    }
}
