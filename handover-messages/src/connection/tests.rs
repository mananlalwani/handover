use super::*;
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::{UnixListener, UnixStream},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

type DaemonFixture = (
    mpsc::Sender<()>,
    mpsc::Sender<()>,
    Arc<Mutex<Option<UnixStream>>>,
    Arc<Mutex<Vec<String>>>,
    thread::JoinHandle<()>,
);

fn mock_daemon(path: std::path::PathBuf) -> DaemonFixture {
    let listener = UnixListener::bind(path).unwrap();
    let (disconnect, disconnected) = mpsc::channel();
    let disconnected = Arc::new(Mutex::new(disconnected));
    let (stop, stopped) = mpsc::channel();
    let subscription = Arc::new(Mutex::new(None));
    let messages_sends = Arc::new(Mutex::new(Vec::new()));
    let subscription_for_server = subscription.clone();
    let sends_for_server = messages_sends.clone();
    let server = thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let subscription_count = Arc::new(Mutex::new(0));
        loop {
            if stopped.try_recv().is_ok() {
                break;
            }
            let stream = match listener.accept() {
                Ok((stream, _)) => stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(_) => break,
            };
            let subscription_count = subscription_count.clone();
            let subscription_for_server = subscription_for_server.clone();
            let sends_for_server = sends_for_server.clone();
            let disconnected = disconnected.clone();
            thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request = String::new();
                if reader.read_line(&mut request).unwrap_or(0) == 0 {
                    return;
                }
                let request: Value = serde_json::from_str(&request).unwrap();
                match request["method"].as_str() {
                    Some("subscribe") => {
                        let number = {
                            let mut count = subscription_count.lock().unwrap();
                            *count += 1;
                            *count
                        };
                        let mut stream = stream;
                        writeln!(
                            stream,
                            "{}",
                            json!({
                                "protocol": 1,
                                "type": "subscribed",
                                "devices": [],
                                "notifications": [],
                                "media_sessions": [],
                                "calls": [],
                                "messaging_accounts": [],
                                "conversations": [],
                                "typing_states": [],
                                "read_states": []
                            })
                        )
                        .unwrap();
                        stream.flush().unwrap();
                        if number == 1 {
                            *subscription_for_server.lock().unwrap() =
                                Some(stream.try_clone().unwrap());
                            let _ = disconnected
                                .lock()
                                .unwrap()
                                .recv_timeout(Duration::from_secs(5));
                        }
                        // Keep each subscription open until the client closes it.
                        let mut discard = String::new();
                        let _ = reader.read_line(&mut discard);
                    }
                    Some("messages.send") => {
                        sends_for_server.lock().unwrap().push(request.to_string());
                    }
                    _ => {}
                }
            });
        }
    });
    (disconnect, stop, subscription, messages_sends, server)
}

fn next_event(receiver: &async_channel::Receiver<Event>) -> Event {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match receiver.try_recv() {
            Ok(event) => return event,
            Err(async_channel::TryRecvError::Closed) => panic!("connection worker stopped"),
            Err(async_channel::TryRecvError::Empty) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(async_channel::TryRecvError::Empty) => panic!("timed out waiting for event"),
        }
    }
}

#[test]
fn send_queued_while_disconnected_fails_before_reconnect_and_is_not_replayed() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("handoverd.sock");
    let (disconnect, stop_server, subscription, messages_sends, server) = mock_daemon(path.clone());
    let (commands, events) = start(path);

    assert!(
        matches!(next_event(&events), Event::Connected(accounts, conversations, _, _)
        if accounts.is_empty() && conversations.is_empty())
    );
    subscription
        .lock()
        .unwrap()
        .take()
        .unwrap()
        .shutdown(std::net::Shutdown::Both)
        .unwrap();
    disconnect.send(()).unwrap();
    assert!(matches!(next_event(&events), Event::Disconnected));

    commands
        .blocking_send(Command {
            id: 42,
            request: Request::Send(MessageDraft {
                conversation_id: ConversationId::new(MessagingAccountId::new("test"), "thread"),
                text: "offline message".into(),
                reply_to: None,
                revision: 0,
            }),
        })
        .unwrap();

    match next_event(&events) {
        Event::Reply(
            42,
            Err(Failure {
                uncertain: false, ..
            }),
        ) => {}
        Event::Connected(_, _, _, _) => {
            panic!("connection re-established before queued command received its offline error")
        }
        _ => panic!("queued command did not receive its offline error"),
    }
    assert!(
        matches!(next_event(&events), Event::Connected(accounts, conversations, _, _)
        if accounts.is_empty() && conversations.is_empty())
    );
    thread::sleep(Duration::from_millis(200));
    assert!(messages_sends.lock().unwrap().is_empty());

    drop(commands);
    drop(events);
    drop(disconnect);
    let _ = stop_server.send(());
    drop(subscription);
    drop(messages_sends);
    let _ = server.join();
}
