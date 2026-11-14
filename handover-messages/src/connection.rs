use handover_core::*;
use handover_ipc::{Client, ErrorCode, IpcError, ServerPayload};
use std::{path::PathBuf, time::Duration};
use tokio::sync::mpsc;

#[derive(Clone)]
pub enum Request {
    Workspace,
    Contacts,
    Sync(MessagingAccountId),
    Select(Option<MessagingLocation>),
    Draft(ConversationId),
    SaveDraft(MessageDraft),
    ResolveDraftSend(MessageDraft, bool),
    AttachmentDraft(ConversationId),
    AttachmentSend(ConversationId, String, String, u64, bool, bool),
    SaveAttachmentQueue(
        ConversationId,
        Vec<handover_core::messaging::AttachmentDraftSource>,
        u64,
    ),
    Search {
        query: String,
        account: Option<MessagingAccountId>,
        offset: u32,
    },
    History(ConversationId, Option<String>),
    Context(MessageId),
    ReplyContext(MessageId),
    UnreadContext(MessageId),
    Quote(MessageId),
    ResolveReply(MessageId),
    React(MessageId, String, bool),
    Previews(Vec<ConversationId>),
    Send(MessageDraft),
    Attachment(MessageDraft, Vec<String>, u64),
    Read(ConversationId, Option<MessageId>),
}

pub enum Reply {
    Workspace(MessagingWorkspace),
    Contacts(Vec<Contact>),
    Draft(MessageDraft),
    AttachmentDraft(MessageAttachmentDraft),
    Search(Vec<MessageSearchHit>, bool),
    History(ConversationId, Vec<Message>, Option<String>),
    Context(ConversationId, Vec<Message>, Option<String>),
    Quote(MessageId, Option<Box<Message>>),
    Resolved(MessageId, Option<Box<Message>>),
    Previews(Vec<Message>),
    Sent(String),
    Accepted,
}

pub struct Failure {
    pub message: String,
    pub code: Option<ErrorCode>,
    pub uncertain: bool,
}

pub enum Event {
    Connected(
        Vec<MessagingAccount>,
        Vec<Conversation>,
        Vec<TypingState>,
        Vec<ReadState>,
    ),
    Disconnected,
    State(Box<ServerPayload>),
    Reply(u64, Result<Reply, Failure>),
}

pub struct Command {
    pub id: u64,
    pub request: Request,
}

pub fn start(path: PathBuf) -> (mpsc::Sender<Command>, async_channel::Receiver<Event>) {
    let (commands, mut incoming) = mpsc::channel::<Command>(16);
    let (events, receiver) = async_channel::bounded(16);
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("messaging client runtime");
        runtime.block_on(async move {
            loop {
                let subscription=tokio::time::timeout(Duration::from_secs(5),async {
                    Client::connect_to(path.clone()).await?.subscribe().await
                }).await;
                let Ok(Ok(subscription))=subscription else {
                    if events.send(Event::Disconnected).await.is_err(){return;}
                    if !retry(&mut incoming,&events).await{return;}
                    continue;
                };
                // The subscription owns framing and stays in one reader task.
                // Never cancel a partially read JSON line to handle a UI command.
                let (updates,mut update)=mpsc::channel(8);
                let accounts=subscription.messaging_accounts.clone();
                let conversations=subscription.conversations.clone();
                let typing=subscription.typing_states.clone();
                let read=subscription.read_states.clone();
                // Nothing admitted by an earlier connection may enter this session.
                while let Ok(command)=incoming.try_recv(){
                    if events.send(Event::Reply(command.id,Err(offline()))).await.is_err(){return;}
                }
                // Send the baseline without moving its live connection to the GTK thread.
                if events.send(Event::Connected(accounts,conversations,typing,read)).await.is_err(){return;}
                let mut subscription=subscription;
                let reader=tokio::spawn(async move {
                    loop {
                        let result=subscription.next_message().await;
                        let stopped=result.is_err();
                        if updates.send(result).await.is_err()||stopped {break;}
                    }
                });
                let mut requests=tokio::task::JoinSet::new();
                let mut active=std::collections::BTreeSet::new();
                loop {
                    tokio::select! {
                        message=update.recv()=>match message {
                            Some(Ok(message))=>if events.send(Event::State(Box::new(message.payload))).await.is_err(){reader.abort();requests.abort_all();return;},
                            _=>break,
                        },
                        command=incoming.recv()=>{
                            let Some(command)=command else {reader.abort();requests.abort_all();return;};
                            if active.len()>=16 {
                                if events.send(Event::Reply(command.id,Err(Failure{message:"Too many requests; try again".into(),code:None,uncertain:false}))).await.is_err(){return;}
                                continue;
                            }
                            active.insert(command.id);
                            let path=path.clone();
                            requests.spawn(async move {(command.id,execute(path,command.request).await)});
                        },
                        result=requests.join_next(),if !requests.is_empty()=>{
                            if let Some(Ok((id,result)))=result {
                                active.remove(&id);
                                if events.send(Event::Reply(id,result)).await.is_err(){reader.abort();requests.abort_all();return;}
                            }
                        }
                    }
                }
                reader.abort();requests.abort_all();
                for id in active {
                    if events.send(Event::Reply(id,Err(Failure{message:"Connection lost; the outcome may be unknown".into(),code:None,uncertain:true}))).await.is_err(){return;}
                }
                if events.send(Event::Disconnected).await.is_err(){return;}
                if !retry(&mut incoming,&events).await{return;}
            }
        });
    });
    (commands, receiver)
}

fn offline() -> Failure {
    Failure {
        message: "Handover is offline".into(),
        code: None,
        uncertain: false,
    }
}

async fn retry(
    incoming: &mut mpsc::Receiver<Command>,
    events: &async_channel::Sender<Event>,
) -> bool {
    let delay = tokio::time::sleep(Duration::from_secs(2));
    tokio::pin!(delay);
    loop {
        tokio::select! {
            _=&mut delay=>return true,
            command=incoming.recv()=>match command {
                Some(command)=>if events.send(Event::Reply(command.id,Err(offline()))).await.is_err(){return false;},
                None=>return false,
            }
        }
    }
}

async fn execute(path: PathBuf, request: Request) -> Result<Reply, Failure> {
    let sending = matches!(
        request,
        Request::Send(_) | Request::Attachment(..) | Request::React(..)
    );
    let result = tokio::time::timeout(Duration::from_secs(255), async {
        let mut client = Client::connect_to(path).await?;
        Ok::<_, IpcError>(match request {
            Request::Workspace => Reply::Workspace(client.messaging_workspace().await?),
            Request::Contacts => Reply::Contacts(client.contacts().await?),
            Request::Sync(account) => {
                client.messaging_sync(account).await?;
                Reply::Accepted
            }
            Request::Select(location) => {
                Reply::Workspace(client.select_message_location(location).await?)
            }
            Request::Draft(id) => Reply::Draft(client.message_draft(id).await?),
            Request::AttachmentDraft(id) => {
                Reply::AttachmentDraft(client.attachment_draft(id).await?)
            }
            Request::SaveAttachmentQueue(id, files, revision) => {
                Reply::AttachmentDraft(client.save_attachment_queue(id, files, revision).await?)
            }
            Request::AttachmentSend(id, operation, caption, revision, resolve, _) => {
                Reply::AttachmentDraft(
                    client
                        .attachment_send(id, operation, caption, revision, resolve)
                        .await?,
                )
            }
            Request::ResolveDraftSend(draft, _) => {
                Reply::Draft(client.resolve_draft_send(draft).await?)
            }
            Request::SaveDraft(draft) => Reply::Draft(client.save_message_draft(draft).await?),
            Request::Search {
                query,
                account,
                offset,
            } => {
                let (hits, more) = client.search_messages(query, account, offset).await?;
                Reply::Search(hits, more)
            }
            Request::History(id, cursor) => {
                let (messages, next) = client
                    .messaging_history(id.clone(), Some(50), cursor)
                    .await?;
                Reply::History(id, messages, next)
            }
            Request::Context(id) | Request::ReplyContext(id) | Request::UnreadContext(id) => {
                let conversation_id = id.conversation_id.clone();
                let (messages, cursor_next) = client.message_context_page(id).await?;
                Reply::Context(conversation_id, messages, cursor_next)
            }
            Request::Quote(id) => {
                let messages = client.message_context(id.clone()).await?;
                let message = messages.into_iter().find(|message| message.id == id);
                Reply::Quote(id, message.map(Box::new))
            }
            Request::ResolveReply(id) => {
                let messages = client.refresh_message_context(id.clone()).await?;
                let message = messages.into_iter().find(|message| message.id == id);
                Reply::Resolved(id, message.map(Box::new))
            }
            Request::React(id, emoji, remove) => {
                if remove {
                    client.unreact_to_message(id, emoji).await?;
                } else {
                    client.react_to_message(id, emoji).await?;
                }
                Reply::Accepted
            }
            Request::Previews(ids) => Reply::Previews(client.message_previews(ids).await?),
            Request::Send(draft) => Reply::Sent(client.submit_message_draft(draft).await?),
            Request::Attachment(draft, urls, _) => {
                let caption = (!draft.text.trim().is_empty()).then_some(draft.text);
                Reply::Sent(if urls.len() == 1 {
                    client
                        .send_message_file(draft.conversation_id, urls[0].clone(), caption)
                        .await?
                } else {
                    client
                        .send_message_files(draft.conversation_id, urls, caption)
                        .await?
                })
            }
            Request::Read(id, message) => {
                client.mark_conversation_read(id, message).await?;
                Reply::Accepted
            }
        })
    })
    .await;
    match result {
        Ok(Ok(reply)) => Ok(reply),
        Ok(Err(IpcError::Server { code, message, .. })) => Err(Failure {
            uncertain: code == ErrorCode::SendOutcomeUnknown,
            code: Some(code),
            message,
        }),
        _ => Err(Failure {
            message: if sending {
                "Send outcome unknown. Check the conversation before sending again."
            } else {
                "Request could not be completed"
            }
            .into(),
            code: None,
            uncertain: sending,
        }),
    }
}

#[cfg(test)]
mod tests;
