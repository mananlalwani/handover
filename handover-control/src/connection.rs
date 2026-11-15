use handover_core::*;
use handover_ipc::{Client, NativePeer, NativePendingPeer, ServerPayload};
use std::{path::PathBuf, time::Duration};
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
pub enum Event {
    Connected(Box<ServerPayload>),
    Disconnected,
    State(Box<ServerPayload>),
    Reply(u64, Result<Reply, String>),
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

pub fn start(path: PathBuf) -> (mpsc::Sender<Command>, async_channel::Receiver<Event>) {
    let (commands, mut incoming) = mpsc::channel::<Command>(8);
    let (events, receiver) = async_channel::bounded(32);
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("control center runtime");
        runtime.block_on(async move {
            loop {
                let connection=tokio::time::timeout(Duration::from_secs(5),async {
                    Client::connect_to(path.clone()).await?.subscribe_control_center().await
                }).await;
                let Ok(Ok(mut subscription))=connection else {
                    if events.send(Event::Disconnected).await.is_err() {return;}
                    let delay=tokio::time::sleep(Duration::from_secs(2));tokio::pin!(delay);
                    loop {tokio::select! {
                        _=&mut delay=>break,
                        command=incoming.recv()=>match command {
                            Some(command)=>{if events.send(Event::Reply(command.id,Err("Handover is offline. Nothing was submitted.".into()))).await.is_err(){return;}},
                            None=>return,
                        }
                    }}
                    continue;
                };
                while let Ok(command)=incoming.try_recv() {
                    if events.send(Event::Reply(command.id,Err("Connection changed. Nothing was submitted.".into()))).await.is_err(){return;}
                }
                let baseline=ServerPayload::Snapshot {
                    devices:std::mem::take(&mut subscription.devices),notifications:std::mem::take(&mut subscription.notifications),
                    media_sessions:std::mem::take(&mut subscription.media_sessions),calls:std::mem::take(&mut subscription.calls),
                    messaging_accounts:vec![],conversations:vec![],typing_states:vec![],read_states:vec![],
                };
                if events.send(Event::Connected(Box::new(baseline))).await.is_err(){return;}
                let (updates,mut update)=mpsc::channel(8);
                let reader=tokio::spawn(async move {
                    loop {
                        let message=subscription.next_message().await;
                        let stopped=message.is_err();
                        if updates.send(message).await.is_err()||stopped {break;}
                    }
                });
                loop {tokio::select! {
                    message=update.recv()=>match message {
                        Some(Ok(message))=>{if events.send(Event::State(Box::new(message.payload))).await.is_err(){reader.abort();return;}},
                        _=>break,
                    },
                    command=incoming.recv()=>{
                        let Some(command)=command else {reader.abort();return;};
                        let mutation=command.request.read_key().is_none();
                        let result=match tokio::time::timeout(Duration::from_secs(12),execute(&path,command.request)).await {
                            Ok(Ok(reply))=>Ok(reply),
                            Ok(Err(handover_ipc::IpcError::Server {code,..}))=>Err(rejection(code).into()),
                            _=>Err(if mutation {"No confirmation received. The outcome is unknown; check the device before trying again."} else {"Could not refresh this view. Reopen it or reconnect to try again."}.into()),
                        };
                        if events.send(Event::Reply(command.id,result)).await.is_err(){reader.abort();return;}
                    }
                }}
                reader.abort();
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
