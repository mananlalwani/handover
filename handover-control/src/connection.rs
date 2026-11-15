use handover_core::*;
use handover_ipc::{Client, NativePeer, NativePendingPeer, ServerPayload};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;

#[derive(Clone)]
pub enum Request {
    Pairing,
    Contacts,
    Transfers,
    Pair(String, String),
    Forget(String),
    Dismiss(NotificationId),
    Action(NotificationId, String),
    SendFile(DeviceId, String),
    SendLink(DeviceId, String),
    Clipboard(DeviceId, String),
    SyncContacts(DeviceId),
    Media(MediaCommand),
}
impl Request {
    pub fn read_key(&self) -> Option<u8> {
        match self {
            Self::Pairing => Some(0),
            Self::Contacts => Some(1),
            Self::Transfers => Some(2),
            _ => None,
        }
    }
}
pub enum Reply {
    Pairing(Vec<NativePeer>, Vec<NativePendingPeer>),
    Contacts(Vec<Contact>),
    Transfers(Vec<TransferActivity>),
    Accepted,
    Share(Option<String>),
}
pub struct Failure {
    pub message: String,
    pub uncertain: bool,
}
impl Failure {
    fn known(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            uncertain: false,
        }
    }
    fn unknown() -> Self {
        Self {message:"No confirmation received. The outcome is unknown; check the device before trying again.".into(),uncertain:true}
    }
}
pub enum Event {
    Connected(Box<ServerPayload>),
    Disconnected,
    State(Box<ServerPayload>),
    Reply(u64, Result<Reply, Failure>),
}
pub struct Command {
    pub id: u64,
    pub request: Request,
}

async fn execute(
    path: &std::path::Path,
    request: Request,
) -> Result<Reply, handover_ipc::IpcError> {
    let mut client = Client::connect_to(path.into()).await?;
    match request {
        Request::Pairing => Ok(Reply::Pairing(
            client.native_peers().await?,
            client.native_pending().await?,
        )),
        Request::Contacts => Ok(Reply::Contacts(client.contacts().await?)),
        Request::Transfers => Ok(Reply::Transfers(client.transfer_activity().await?)),
        Request::Pair(id, code) => {
            client.native_pair(id, code).await?;
            Ok(Reply::Accepted)
        }
        Request::Forget(id) => {
            client.native_unpair(id).await?;
            Ok(Reply::Accepted)
        }
        Request::Dismiss(id) => {
            client.dismiss_notification(id).await?;
            Ok(Reply::Accepted)
        }
        Request::Action(id, action) => {
            client.invoke_notification_action(id, action).await?;
            Ok(Reply::Accepted)
        }
        Request::SendFile(id, url) => {
            Ok(Reply::Share(client.send_file_url_tracked(id, url).await?))
        }
        Request::SendLink(id, url) => Ok(Reply::Share(client.send_url_tracked(id, url).await?)),
        Request::Clipboard(id, text) => {
            client.send_clipboard(id, text).await?;
            Ok(Reply::Accepted)
        }
        Request::SyncContacts(id) => {
            client.sync_contacts(id).await?;
            Ok(Reply::Accepted)
        }
        Request::Media(command) => {
            client.media_command(command).await?;
            Ok(Reply::Accepted)
        }
    }
}

async fn run_request(path: &std::path::Path, request: Request) -> Result<Reply, Failure> {
    let mutation = request.read_key().is_none();
    match tokio::time::timeout(Duration::from_secs(12), execute(path, request)).await {
        Ok(Ok(reply)) => Ok(reply),
        Ok(Err(handover_ipc::IpcError::Server {
            code: handover_ipc::ErrorCode::SendOutcomeUnknown,
            ..
        })) => Err(Failure::unknown()),
        Ok(Err(handover_ipc::IpcError::Server { code, .. })) => {
            Err(Failure::known(rejection(code)))
        }
        _ => Err(if mutation {
            Failure::unknown()
        } else {
            Failure::known("Could not refresh this view. Reopen it or reconnect to try again.")
        }),
    }
}

pub fn start(path: PathBuf) -> (mpsc::Sender<Command>, async_channel::Receiver<Event>) {
    let (commands, mut incoming) = mpsc::channel::<Command>(16);
    let (events, receiver) = async_channel::bounded(32);
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("control center runtime");
        runtime.block_on(async move {
            let session=Arc::new(AtomicU64::new(0));
            let (execution,mut requests)=mpsc::channel::<(u64,Command)>(16);
            let worker_session=session.clone();let worker_events=events.clone();let worker_path=path.clone();
            tokio::spawn(async move {
                while let Some((generation,command))=requests.recv().await {
                    // Queued work from a lost subscription is rejected, never replayed in a new session.
                    let read=command.request.read_key().is_some();
                    let mut result=if worker_session.load(Ordering::Acquire)==generation {
                        run_request(&worker_path,command.request).await
                    }else{Err(Failure::known("Connection changed. Nothing was submitted."))};
                    if read&&worker_session.load(Ordering::Acquire)!=generation {result=Err(Failure::known("Connection changed. Refresh this view to try again."));}
                    if worker_events.send(Event::Reply(command.id,result)).await.is_err(){break;}
                }
            });
            let mut generation=0u64;
            loop {
                let connection=tokio::time::timeout(Duration::from_secs(5),async {
                    Client::connect_to(path.clone()).await?.subscribe_control_center().await
                }).await;
                let Ok(Ok(mut subscription))=connection else {
                    if events.send(Event::Disconnected).await.is_err(){return;}
                    let delay=tokio::time::sleep(Duration::from_secs(2));tokio::pin!(delay);
                    loop {tokio::select! {
                        _=&mut delay=>break,
                        command=incoming.recv()=>match command {
                            Some(command)=>{if events.send(Event::Reply(command.id,Err(Failure::known("Handover is offline. Nothing was submitted.")))).await.is_err(){return;}},
                            None=>return,
                        }
                    }}
                    continue;
                };
                while let Ok(command)=incoming.try_recv(){if events.send(Event::Reply(command.id,Err(Failure::known("Connection changed. Nothing was submitted.")))).await.is_err(){return;}}
                let Some(next)=generation.checked_add(1) else{return;};generation=next;session.store(generation,Ordering::Release);
                let baseline=ServerPayload::Snapshot {
                    devices:std::mem::take(&mut subscription.devices),notifications:std::mem::take(&mut subscription.notifications),media_sessions:std::mem::take(&mut subscription.media_sessions),calls:std::mem::take(&mut subscription.calls),messaging_accounts:vec![],conversations:vec![],typing_states:vec![],read_states:vec![],
                };
                if events.send(Event::Connected(Box::new(baseline))).await.is_err(){return;}
                let (updates,mut update)=mpsc::channel(8);
                let reader_session=session.clone();
                let reader=tokio::spawn(async move {
                    loop {
                        let message=subscription.next_message().await;
                        let stopped=message.is_err();
                        if stopped {reader_session.store(0,Ordering::Release);}
                        if updates.send(message).await.is_err()||stopped {break;}
                    }
                });
                loop {tokio::select! {
                    message=update.recv()=>match message {
                        Some(Ok(message))=>{if events.send(Event::State(Box::new(message.payload))).await.is_err(){reader.abort();return;}},
                        _=>break,
                    },
                    command=incoming.recv()=>{
                        let Some(command)=command else{reader.abort();return;};
                        if let Err(error)=execution.try_send((generation,command)) {
                            let (_,command)=error.into_inner();
                            if events.send(Event::Reply(command.id,Err(Failure::known("Handover is busy. Nothing was submitted; try again shortly.")))).await.is_err(){reader.abort();return;}
                        }
                    }
                }}
                session.store(0,Ordering::Release);reader.abort();
                if events.send(Event::Disconnected).await.is_err(){return;}
            }
        });
    });
    (commands, receiver)
}

fn rejection(code: handover_ipc::ErrorCode) -> &'static str {
    use handover_ipc::ErrorCode;
    match code {
        ErrorCode::UnknownDevice | ErrorCode::DeviceDisconnected => {
            "Phone disconnected. Reconnect and try again."
        }
        ErrorCode::DeviceNotPaired => "Pair this device before using this action.",
        ErrorCode::UnsupportedCapability => "This action is not available on this device.",
        ErrorCode::NotificationNotFound => "This notification is no longer available.",
        ErrorCode::BackendUnavailable => "The device service is unavailable.",
        _ => "The request was rejected. Check the device and try again.",
    }
}
