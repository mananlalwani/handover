use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{CommandFactory, Parser};
use handover_core::{
    Contact, ConversationId, Device, DeviceId, MediaCommand, MediaSession, MediaSessionId,
    MessageId, MessagingAccountId, Notification, SharedResource,
};
use handover_ipc::{Client, IpcError, PROTOCOL_VERSION, ServerPayload};
use handoverctl::Cli;
use handoverctl::cli::{
    ClipboardHistoryCommand, Command, ContactsCommand, CustomCommand, MediaSubcommand,
    MessagesCommand, NativeCommand,
};
use thiserror::Error;
use url::Url;

#[derive(Debug, Error)]
enum CliError {
    #[error(transparent)]
    Ipc(#[from] IpcError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    DeviceSelection(String),
    #[error("{0}")]
    MediaSelection(String),
    #[error("{0}")]
    MessagingSelection(String),
    #[error("cannot convert local path to a file URL")]
    InvalidFilePath,
    #[error("credential bundle is empty or too large (max 192 KiB raw)")]
    InvalidBundle,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Some(Command::Devices) => list_devices().await,
        Some(Command::Native { command }) => native(command).await,
        Some(Command::Notifications) => list_notifications().await,
        Some(Command::Contacts { command }) => contacts(command).await,
        Some(Command::Media { command }) => media(command).await,
        Some(Command::Monitor) => monitor().await,
        Some(Command::SendUrl { device, url }) => send_url(&device, url).await,
        Some(Command::Notify {
            device,
            app,
            title,
            body,
        }) => send_notification(&device, app, title, body).await,
        Some(Command::Clipboard { device, text }) => send_clipboard(&device, text).await,
        Some(Command::SendFile { device, path }) => send_file(&device, path).await,
        Some(Command::Screensaver { action }) => screensaver(&action).await,
        Some(Command::ClipboardMirror { action }) => clipboard_mirror(&action).await,
        Some(Command::ClipboardHistory { command }) => clipboard_history(command).await,
        Some(Command::CancelShare {
            device,
            transfer_id,
        }) => cancel_share(&device, transfer_id).await,
        Some(Command::Custom { command }) => custom(command).await,
        Some(Command::Messages { command }) => messages(command).await,
        Some(Command::Calls { device }) => calls(device).await,
        None => {
            Cli::command()
                .print_help()
                .expect("writing help to stdout should succeed");
            println!();
            return ExitCode::SUCCESS;
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("handoverctl: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn send_clipboard(selector: &str, text: Option<String>) -> Result<(), CliError> {
    let mut client = connected_client().await?;
    let device = select_device(&client.devices().await?, selector)?;
    if let Some(text) = text {
        client.send_clipboard(device, text).await?;
    } else {
        client.send_current_clipboard(device).await?;
    }
    println!("Clipboard accepted by phone");
    Ok(())
}

async fn contacts(command: ContactsCommand) -> Result<(), CliError> {
    let mut client = connected_client().await?;
    match command {
        ContactsCommand::List => print_contacts(&client.contacts().await?),
        ContactsCommand::Sync { device } => {
            let devices = client.devices().await?;
            let device_id = select_device(&devices, &device)?;
            let name = devices
                .iter()
                .find(|candidate| candidate.id == device_id)
                .map(|candidate| candidate.name.clone())
                .unwrap_or_else(|| device_id.to_string());
            client.sync_contacts(device_id).await?;
            println!("Contacts sync requested for {name}");
        }
    }
    Ok(())
}

fn print_contacts(contacts: &[Contact]) {
    for contact in contacts {
        let phones = contact.phones.join(", ");
        let emails = contact.emails.join(", ");
        println!(
            "{}\t{}\t{}\t{}\tphoto={}",
            contact.device_id,
            contact.display_name,
            phones,
            emails,
            if contact.photo.is_some() { "yes" } else { "no" }
        );
    }
}

async fn native(command: NativeCommand) -> Result<(), CliError> {
    let mut client = connected_client().await?;
    match command {
        NativeCommand::Peers => {
            for peer in client.native_peers().await? {
                println!("{}\t{}\t{}", peer.id, peer.name, peer.fingerprint);
            }
        }
        NativeCommand::Pending => {
            for peer in client.native_pending().await? {
                println!("{}\t{}\t{}", peer.id, peer.name, peer.code);
            }
        }
        NativeCommand::Pair { id, code } => {
            client.native_pair(id, code).await?;
            println!("Pair approval recorded; waiting for phone confirmation");
        }
        NativeCommand::Unpair { id } => {
            client.native_unpair(id).await?;
            println!("Native peer revoked");
        }
        NativeCommand::Ping { device } => {
            let id = select_native_peer(&mut client, &device).await?;
            client.native_ping(id).await?;
            println!("Ping queued; use monitor to observe the Android result");
        }
        NativeCommand::Ring { device } => {
            let id = select_native_peer(&mut client, &device).await?;
            client.native_ring(id).await?;
            println!("Ring queued; use monitor to observe the Android result");
        }
        NativeCommand::Lock { device } => {
            let id = select_native_peer(&mut client, &device).await?;
            client.native_lock(id).await?;
            println!("Lock queued; use monitor to observe the Android result");
        }
        NativeCommand::KeepAwake { device, release } => {
            let id = select_native_peer(&mut client, &device).await?;
            client.native_keep_awake(id, !release).await?;
            println!("Keep-awake queued; use monitor to observe the Android result");
        }
        NativeCommand::Tethering { device } => {
            let id = select_native_peer(&mut client, &device).await?;
            client.native_tethering(id).await?;
            println!("Tethering settings queued; use monitor to observe the Android result");
        }
        NativeCommand::Call {
            device,
            action,
            address,
            confirm,
        } => {
            let id = select_native_peer(&mut client, &device).await?;
            if action == "place" && !confirm {
                return Err(CliError::DeviceSelection(
                    "placing a real call requires --confirm".into(),
                ));
            }
            client.native_call(id, action, address).await?;
            println!("Call command accepted; effect is not confirmed");
        }
    }
    Ok(())
}

async fn list_devices() -> Result<(), CliError> {
    let mut client = connected_client().await?;
    let devices = client.devices().await?;
    print_table(&devices);
    Ok(())
}

async fn calls(device: String) -> Result<(), CliError> {
    let mut client = connected_client().await?;
    let devices = client.devices().await?;
    let id = select_device(&devices, &device)?;
    let calls = client.calls().await?;
    match calls.into_iter().find(|call| call.device_id == id) {
        Some(call) => {
            let mut actions = call
                .controls
                .iter()
                .map(|action| action.as_str())
                .collect::<Vec<_>>();
            actions.sort_unstable();
            println!(
                "{}\t{:?}\tavailable: {}",
                call.device_id,
                call.phase,
                if actions.is_empty() {
                    "none".into()
                } else {
                    actions.join(", ")
                }
            );
        }
        None => println!("no call state attested for {id}"),
    }
    Ok(())
}

async fn list_notifications() -> Result<(), CliError> {
    let mut client = connected_client().await?;
    let devices = client.devices().await?;
    let notifications = client.notifications().await?;
    print_notification_table(&devices, &notifications);
    Ok(())
}

async fn media(command: Option<MediaSubcommand>) -> Result<(), CliError> {
    let mut client = connected_client().await?;
    let devices = client.devices().await?;
    let sessions = client.media_sessions().await?;

    let Some(command) = command else {
        print_media_table(&devices, &sessions);
        return Ok(());
    };

    let (selector, command) = match command {
        MediaSubcommand::Play { session } => (session, MediaAction::Play),
        MediaSubcommand::Pause { session } => (session, MediaAction::Pause),
        MediaSubcommand::PlayPause { session } => (session, MediaAction::PlayPause),
        MediaSubcommand::Next { session } => (session, MediaAction::Next),
        MediaSubcommand::Previous { session } => (session, MediaAction::Previous),
    };
    let id = select_media_session(&sessions, &selector)?;
    client
        .media_command(command.into_command(id.clone()))
        .await?;
    println!("media command accepted for {}", media_selector(&id));
    Ok(())
}

#[derive(Clone, Copy)]
enum MediaAction {
    Play,
    Pause,
    PlayPause,
    Next,
    Previous,
}

impl MediaAction {
    fn into_command(self, id: MediaSessionId) -> MediaCommand {
        match self {
            Self::Play => MediaCommand::Play { id },
            Self::Pause => MediaCommand::Pause { id },
            Self::PlayPause => MediaCommand::PlayPause { id },
            Self::Next => MediaCommand::Next { id },
            Self::Previous => MediaCommand::Previous { id },
        }
    }
}

async fn monitor() -> Result<(), CliError> {
    loop {
        match connected_client().await {
            Ok(client) => match client.subscribe().await {
                Ok(mut subscription) => {
                    println!(
                        "subscribed: {} device(s), {} notification(s)",
                        subscription.devices.len(),
                        subscription.notifications.len()
                    );
                    loop {
                        tokio::select! {
                            message = subscription.next_message() => {
                                match message {
                                    Ok(message) => print_message(message.payload),
                                    Err(error) => {
                                        eprintln!("handoverctl: connection lost: {error}; retrying");
                                        break;
                                    }
                                }
                            }
                            result = tokio::signal::ctrl_c() => {
                                result.map_err(IpcError::Io)?;
                                return Ok(());
                            }
                        }
                    }
                }
                Err(error) => eprintln!("handoverctl: subscription failed: {error}; retrying"),
            },
            Err(error) => eprintln!("handoverctl: daemon unavailable: {error}; retrying"),
        }

        tokio::select! {
            () = tokio::time::sleep(Duration::from_secs(2)) => {}
            result = tokio::signal::ctrl_c() => {
                result.map_err(IpcError::Io)?;
                return Ok(());
            }
        }
    }
}

async fn send_url(selector: &str, url: String) -> Result<(), CliError> {
    let mut client = connected_client().await?;
    let device_id = select_device(&client.devices().await?, selector)?;
    let transfer_id = client.send_url_tracked(device_id, url).await?;
    match transfer_id {
        Some(id) => println!("URL share accepted: transfer {id}; awaiting receiver result"),
        None => println!("URL share accepted; delivery is not confirmed"),
    }
    Ok(())
}

async fn send_notification(
    selector: &str,
    app: String,
    title: String,
    body: String,
) -> Result<(), CliError> {
    let mut client = connected_client().await?;
    let device_id = select_device(&client.devices().await?, selector)?;
    client
        .send_notification(device_id, app, title, body)
        .await?;
    println!("Notification queued; use monitor to observe the Android result");
    Ok(())
}

async fn send_file(selector: &str, path: PathBuf) -> Result<(), CliError> {
    let mut client = connected_client().await?;
    let device_id = select_device(&client.devices().await?, selector)?;
    let absolute = tokio::fs::canonicalize(path).await?;
    let file_url = Url::from_file_path(absolute).map_err(|_| CliError::InvalidFilePath)?;
    let transfer_id = client
        .send_file_url_tracked(device_id, file_url.into())
        .await?;
    match transfer_id {
        Some(id) => println!("File share accepted: transfer {id}; awaiting receiver result"),
        None => println!("File share accepted; delivery is not confirmed"),
    }
    Ok(())
}

async fn screensaver(action: &str) -> Result<(), CliError> {
    let mut client = connected_client().await?;
    match action {
        "inhibit" => client.set_screensaver(Some(true)).await?,
        "release" => client.set_screensaver(Some(false)).await?,
        "follow" => client.set_screensaver(None).await?,
        _ => {
            return Err(CliError::DeviceSelection(
                "screensaver action must be inhibit, release, or follow".into(),
            ));
        }
    }
    println!("Desktop inhibitor override: {action}");
    Ok(())
}

async fn custom(command: CustomCommand) -> Result<(), CliError> {
    let mut client = connected_client().await?;
    match command {
        CustomCommand::List => {
            println!("NAME  ARGV");
            for entry in client.custom_commands().await? {
                println!("{}  {}", entry.name, entry.argv.join(" "));
            }
        }
        CustomCommand::Run { name } => {
            let result = client.run_custom(name).await?;
            println!(
                "custom command {}: accepted={} exit={:?} failure={:?}",
                result.name, result.accepted, result.exit_code, result.failure
            );
        }
    }
    Ok(())
}

async fn clipboard_mirror(action: &str) -> Result<(), CliError> {
    let mut client = connected_client().await?;
    match action {
        "on" => {
            client.set_clipboard_mirror(true).await?;
            println!("Background clipboard mirroring: on (text only)");
        }
        "off" => {
            client.set_clipboard_mirror(false).await?;
            println!("Background clipboard mirroring: off");
        }
        "status" => {
            let enabled = client.clipboard_mirror_status().await?;
            println!(
                "Background clipboard mirroring: {}",
                if enabled { "on" } else { "off" }
            );
        }
        _ => {
            return Err(CliError::DeviceSelection(
                "clipboard mirror action must be on, off, or status".into(),
            ));
        }
    }
    Ok(())
}

async fn clipboard_history(command: ClipboardHistoryCommand) -> Result<(), CliError> {
    let mut client = Client::connect().await?;
    match command {
        ClipboardHistoryCommand::List => {
            let entries = client.clipboard_history().await?;
            if entries.is_empty() {
                println!("No phone clipboard history");
            } else {
                println!("ID\tPINNED\tTEXT");
                for entry in entries {
                    let mut preview: String = entry.text.escape_default().take(120).collect();
                    if preview.len() < entry.text.escape_default().count() {
                        preview.push_str("...");
                    }
                    println!("{}\t{}\t{}", entry.id, entry.pinned, preview);
                }
            }
        }
        ClipboardHistoryCommand::Save { text } => {
            client.save_clipboard_history(text).await?;
            println!("Pinned clipboard string saved");
        }
        ClipboardHistoryCommand::Pin { id } => {
            client.set_clipboard_history_pin(id, true).await?;
            println!("Clipboard history entry {id} pinned");
        }
        ClipboardHistoryCommand::Unpin { id } => {
            client.set_clipboard_history_pin(id, false).await?;
            println!("Clipboard history entry {id} unpinned");
        }
        ClipboardHistoryCommand::Copy { id } => {
            client.copy_clipboard_history(id).await?;
            println!("Clipboard history entry {id} copied");
        }
        ClipboardHistoryCommand::Clear { all } => {
            client.clear_clipboard_history(all).await?;
            println!(
                "{}",
                if all {
                    "Clipboard history cleared"
                } else {
                    "Recent clipboard history cleared; pinned entries retained"
                }
            );
        }
    }
    Ok(())
}

async fn cancel_share(selector: &str, transfer_id: String) -> Result<(), CliError> {
    let mut client = connected_client().await?;
    let device_id = select_device(&client.devices().await?, selector)?;
    if client.cancel_share(device_id, transfer_id).await? {
        println!("Share cancelled before streaming started");
    } else {
        println!("Share was already sent or is unknown; receiver result stands");
    }
    Ok(())
}

async fn messages(command: MessagesCommand) -> Result<(), CliError> {
    let mut client = connected_client().await?;
    match command {
        MessagesCommand::Accounts => {
            println!("ACCOUNT  LABEL  STATE");
            for account in client.messaging_accounts().await? {
                let state = match (account.connected, account.authenticated) {
                    (true, true) => "online",
                    (true, false) => "pairing",
                    (false, _) => "offline",
                };
                println!("{}\t{}\t{state}", account.id, account.label);
            }
        }
        MessagesCommand::Conversations { account } => {
            let account_id = MessagingAccountId::new(account);
            let conversations = client.messaging_conversations(account_id).await?;
            println!("CONVERSATION  KIND  TRANSPORT  TITLE/PARTICIPANTS  UNREAD");
            for conversation in conversations {
                let title = conversation_title(&conversation);
                let unread = conversation
                    .unread_count
                    .map_or("-".into(), |count| count.to_string());
                println!(
                    "{}\t{:?}\t{:?}\t{title}\t{unread}",
                    conversation.id, conversation.kind, conversation.transport
                );
            }
        }
        MessagesCommand::History {
            conversation,
            limit,
            cursor,
        } => {
            let conversation_id = resolve_conversation(&mut client, &conversation).await?;
            let (messages, next) = client
                .messaging_history(conversation_id.clone(), limit, cursor)
                .await?;
            for message in &messages {
                print_message_record(&conversation_id, message);
            }
            match next {
                Some(cursor) => println!("-- older available: --cursor {cursor}"),
                None => println!("-- start of stored window --"),
            }
        }
        MessagesCommand::Send { conversation, text } => {
            let conversation_id = resolve_conversation(&mut client, &conversation).await?;
            let request_id = client.send_message_text(conversation_id, text).await?;
            println!("send accepted: request {request_id}; delivery is not confirmed");
        }
        MessagesCommand::SendFile {
            conversation,
            path,
            caption,
        } => {
            let conversation_id = resolve_conversation(&mut client, &conversation).await?;
            let absolute = tokio::fs::canonicalize(path).await?;
            let file_url = Url::from_file_path(absolute).map_err(|_| CliError::InvalidFilePath)?;
            let request_id = client
                .send_message_file(conversation_id, file_url.into(), caption)
                .await?;
            println!("attachment accepted: request {request_id}; delivery is not confirmed");
        }
        MessagesCommand::Reply { message, text } => {
            // Replies address the origin conversation; the daemon validates
            // that the target message is known before accepting.
            let message_id = resolve_message(&mut client, &message).await?;
            let request_id = client.send_message_reply(message_id.clone(), text).await?;
            println!(
                "reply accepted for {}: request {request_id}; delivery is not confirmed",
                message_id.local_id
            );
        }
        MessagesCommand::React { message, emoji } => {
            let message_id = resolve_message(&mut client, &message).await?;
            let request_id = client.react_to_message(message_id, emoji).await?;
            println!("reaction accepted: request {request_id}");
        }
        MessagesCommand::Unreact { message, emoji } => {
            let message_id = resolve_message(&mut client, &message).await?;
            let request_id = client.unreact_to_message(message_id, emoji).await?;
            println!("reaction removal accepted: request {request_id}");
        }
        MessagesCommand::Read {
            conversation,
            message,
        } => {
            let conversation_id = resolve_conversation(&mut client, &conversation).await?;
            let message_id = match message {
                Some(selector) => Some(resolve_message(&mut client, &selector).await?),
                None => None,
            };
            client
                .mark_conversation_read(conversation_id.clone(), message_id)
                .await?;
            println!("read queued for {conversation_id}");
        }
        MessagesCommand::Typing { conversation } => {
            let conversation_id = resolve_conversation(&mut client, &conversation).await?;
            client.start_typing(conversation_id.clone()).await?;
            println!("typing-start queued for {conversation_id}");
        }
        MessagesCommand::Delete { message } => {
            let message_id = resolve_message(&mut client, &message).await?;
            let request_id = client.delete_message(message_id).await?;
            println!("delete accepted: request {request_id}");
        }
        MessagesCommand::Open { account, addresses } => {
            let request_id = client
                .open_conversation(MessagingAccountId::new(account), addresses)
                .await?;
            println!("open accepted: request {request_id}; watch for the conversation event");
        }
        MessagesCommand::Login { account, from_file } => {
            let bundle = read_bundle(from_file).await?;
            if bundle.is_empty() || bundle.len() as u64 > MAX_RAW_BUNDLE_BYTES {
                return Err(CliError::InvalidBundle);
            }
            client
                .messaging_login(
                    MessagingAccountId::new(account.clone()),
                    base64_encode(&bundle),
                )
                .await?;
            println!("login accepted for {account}; confirm pairing on the phone");
        }
        MessagesCommand::Logout { account } => {
            client
                .messaging_logout(MessagingAccountId::new(account.clone()))
                .await?;
            println!("logout queued for {account}; access ends on revoke");
        }
        MessagesCommand::Sync { account } => {
            client
                .messaging_sync(MessagingAccountId::new(account.clone()))
                .await?;
            println!("sync requested for {account}");
        }
    }
    Ok(())
}

/// Maximum raw bundle bytes the CLI reads. Bundles are base64-encoded
/// before crossing IPC, and the daemon caps the encoded form at
/// 256 KiB; 192 KiB raw encodes to exactly that cap. Reads stop one
/// byte past the limit so oversize input is rejected, not buffered.
const MAX_RAW_BUNDLE_BYTES: u64 = 192 * 1024;

/// Read a credential bundle from a file or stdin. Bundles never travel
/// through argv and are never printed.
async fn read_bundle(from_file: Option<PathBuf>) -> Result<Vec<u8>, CliError> {
    use tokio::io::AsyncReadExt;
    match from_file {
        Some(path) => {
            let file = tokio::fs::File::open(path).await.map_err(CliError::Io)?;
            let mut bundle = Vec::new();
            file.take(MAX_RAW_BUNDLE_BYTES + 1)
                .read_to_end(&mut bundle)
                .await
                .map_err(CliError::Io)?;
            Ok(bundle)
        }
        None => {
            let bundle = tokio::task::spawn_blocking(|| {
                use std::io::Read;
                let mut bundle = Vec::new();
                std::io::stdin()
                    .take(MAX_RAW_BUNDLE_BYTES + 1)
                    .read_to_end(&mut bundle)
                    .map(|_| bundle)
            })
            .await
            .map_err(|_| CliError::InvalidBundle)?
            .map_err(CliError::Io)?;
            Ok(bundle)
        }
    }
}

fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let mut block = [0u8; 3];
        block[..chunk.len()].copy_from_slice(chunk);
        let value = (u32::from(block[0]) << 16) | (u32::from(block[1]) << 8) | u32::from(block[2]);
        output.push(ALPHABET[(value >> 18) as usize & 63] as char);
        output.push(ALPHABET[(value >> 12) as usize & 63] as char);
        output.push(if chunk.len() > 1 {
            ALPHABET[(value >> 6) as usize & 63] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            ALPHABET[value as usize & 63] as char
        } else {
            '='
        });
    }
    output
}

/// Resolve `ACCOUNT:THREAD` or a bare `THREAD` (when unambiguous across
/// accounts) to a conversation id. Account ids themselves contain colons,
/// so qualification matches the longest known account prefix instead of
/// splitting on the first colon.
async fn resolve_conversation(
    client: &mut Client,
    selector: &str,
) -> Result<ConversationId, CliError> {
    let accounts = client.messaging_accounts().await?;
    let mut qualified: Option<(MessagingAccountId, &str)> = None;
    for account in &accounts {
        let prefix = format!("{}:", account.id);
        if let Some(thread) = selector.strip_prefix(&prefix) {
            if thread.is_empty() {
                continue;
            }
            match &qualified {
                Some((_, known)) if known.len() >= thread.len() => {}
                _ => qualified = Some((account.id.clone(), thread)),
            }
        }
    }
    if let Some((account_id, thread)) = qualified {
        return Ok(ConversationId::new(account_id, thread));
    }
    if selector.contains(':') {
        return Err(CliError::MessagingSelection(format!(
            "no messaging account matches {selector:?}"
        )));
    }
    let mut matches = Vec::new();
    for account in &accounts {
        for conversation in client.messaging_conversations(account.id.clone()).await? {
            if conversation.id.local_id == selector {
                matches.push(conversation.id);
            }
        }
    }
    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => Err(CliError::MessagingSelection(format!(
            "no conversation identified by {selector:?}"
        ))),
        _ => Err(CliError::MessagingSelection(format!(
            "multiple conversations match {selector:?}; use ACCOUNT:THREAD"
        ))),
    }
}

/// Resolve `ACCOUNT:THREAD:MESSAGE`, `THREAD:MESSAGE`, or a bare message id
/// (when unambiguous) to a message id. Qualification strips the longest
/// known account prefix first; the remainder splits on the last colon.
async fn resolve_message(client: &mut Client, selector: &str) -> Result<MessageId, CliError> {
    let accounts = client.messaging_accounts().await?;
    // Strip the longest known account prefix; account ids contain colons,
    // so splitting on a fixed position would misparse.
    let mut rest = selector;
    let mut remainder_account: Option<MessagingAccountId> = None;
    for account in &accounts {
        let prefix = format!("{}:", account.id);
        if let Some(remainder) = selector.strip_prefix(&prefix) {
            if remainder.is_empty() {
                continue;
            }
            let longer = remainder_account
                .as_ref()
                .is_some_and(|known: &MessagingAccountId| {
                    selector
                        .strip_prefix(format!("{}:", known).as_str())
                        .is_some_and(|known_rest| known_rest.len() >= remainder.len())
                });
            if !longer {
                remainder_account = Some(account.id.clone());
                rest = remainder;
            }
        }
    }
    let (thread, message) = match rest.rsplit_once(':') {
        Some((thread, message)) if !message.is_empty() => (Some(thread), message),
        _ => (None, rest),
    };
    if message.is_empty() {
        return Err(CliError::MessagingSelection(format!(
            "no message identified by {selector:?}"
        )));
    }
    let mut matches = Vec::new();
    for known_account in client.messaging_accounts().await? {
        if let Some(expected) = &remainder_account {
            if &known_account.id != expected {
                continue;
            }
        }
        for conversation in client
            .messaging_conversations(known_account.id.clone())
            .await?
        {
            if let Some(thread) = thread {
                if conversation.id.local_id != thread {
                    continue;
                }
            }
            let (history, _) = client
                .messaging_history(conversation.id.clone(), Some(100), None)
                .await?;
            for known in history {
                if known.id.local_id == message {
                    matches.push(known.id);
                }
            }
        }
    }
    // Deduplicate: the same id can surface through overlapping scans.
    matches.sort();
    matches.dedup();
    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => Err(CliError::MessagingSelection(format!(
            "no message identified by {selector:?}"
        ))),
        _ => Err(CliError::MessagingSelection(format!(
            "multiple messages match {selector:?}; use ACCOUNT:THREAD:MESSAGE"
        ))),
    }
}

fn conversation_title(conversation: &handover_core::Conversation) -> String {
    if let Some(title) = &conversation.title {
        return title.clone();
    }
    let others: Vec<String> = conversation
        .participants
        .iter()
        .filter(|participant| !participant.is_self)
        .map(|participant| {
            participant
                .display_name
                .clone()
                .or_else(|| participant.address.clone())
                .unwrap_or_else(|| participant.local_id.clone())
        })
        .collect();
    others.join(", ")
}

fn print_message_record(conversation_id: &ConversationId, message: &handover_core::Message) {
    let sender = message
        .sender
        .display_name
        .clone()
        .or_else(|| message.sender.address.clone())
        .unwrap_or_else(|| message.sender.local_id.clone());
    let marker = if message.sender.is_self { "me" } else { "them" };
    let body = message.text.as_deref().unwrap_or("");
    let attachments: Vec<String> = message
        .attachments
        .iter()
        .map(|attachment| {
            attachment
                .name
                .clone()
                .unwrap_or_else(|| attachment.local_id.clone())
        })
        .collect();
    let attachment_suffix = if attachments.is_empty() {
        String::new()
    } else {
        format!(" [{}]", attachments.join(", "))
    };
    let reply_suffix = message
        .reply_to
        .as_ref()
        .map(|reply| format!(" (reply to {})", reply.local_id))
        .unwrap_or_default();
    let reaction_suffix = if message.reactions.is_empty() {
        String::new()
    } else {
        let reactions: Vec<String> = message
            .reactions
            .iter()
            .map(|reaction| format!("{}×{}", reaction.emoji, reaction.count))
            .collect();
        format!(" {{{}}}", reactions.join(" "))
    };
    let deleted = if message.deleted { " [deleted]" } else { "" };
    println!(
        "{} [{}] {sender} ({marker}): {body}{attachment_suffix}{reply_suffix}{reaction_suffix}{deleted}",
        message.id.local_id, conversation_id.local_id
    );
}

fn select_device(devices: &[Device], selector: &str) -> Result<DeviceId, CliError> {
    if let Some(device) = devices.iter().find(|device| device.id.as_str() == selector) {
        return Ok(device.id.clone());
    }
    let mut matches = devices.iter().filter(|device| device.name == selector);
    let first = matches.next().ok_or_else(|| {
        CliError::DeviceSelection(format!("no device named or identified by {selector:?}"))
    })?;
    if matches.next().is_some() {
        return Err(CliError::DeviceSelection(format!(
            "multiple devices are named {selector:?}; use a device ID"
        )));
    }
    Ok(first.id.clone())
}

async fn select_native_peer(client: &mut Client, selector: &str) -> Result<String, CliError> {
    let peers = client.native_peers().await?;
    select_native_peer_from_slice(&peers, selector)
}

fn select_native_peer_from_slice(
    peers: &[handover_ipc::NativePeer],
    selector: &str,
) -> Result<String, CliError> {
    if let Some(peer) = peers.iter().find(|peer| peer.id == selector) {
        return Ok(peer.id.clone());
    }
    let mut matches = peers.iter().filter(|peer| peer.name == selector);
    let first = matches.next().ok_or_else(|| {
        CliError::DeviceSelection(format!(
            "no native device named or identified by {selector:?}"
        ))
    })?;
    if matches.next().is_some() {
        return Err(CliError::DeviceSelection(format!(
            "multiple native devices are named {selector:?}; use a peer ID"
        )));
    }
    Ok(first.id.clone())
}

async fn connected_client() -> Result<Client, IpcError> {
    let mut client = Client::connect().await?;
    let supported = client.hello().await?;
    if !supported.contains(&PROTOCOL_VERSION) {
        return Err(IpcError::UnexpectedResponse(format!(
            "daemon does not support protocol {PROTOCOL_VERSION}"
        )));
    }
    Ok(client)
}

fn print_table(devices: &[Device]) {
    let name_width = devices
        .iter()
        .map(|device| device.name.chars().count())
        .max()
        .unwrap_or(6)
        .max(6);
    println!(
        "{:<name_width$}  {:<12}  {:<8}  BATTERY",
        "DEVICE", "STATUS", "PAIRED"
    );
    for device in devices {
        let status = if device.connected {
            "connected"
        } else {
            "offline"
        };
        let paired = if device.paired { "yes" } else { "no" };
        let battery = device
            .battery
            .map(|battery| format!("{}%", battery.percentage()))
            .unwrap_or_else(|| "-".into());
        println!(
            "{:<name_width$}  {:<12}  {:<8}  {}",
            device.name, status, paired, battery
        );
    }
}

fn print_notification_table(devices: &[Device], notifications: &[Notification]) {
    println!("DEVICE  APP  TITLE");
    for notification in notifications {
        let device_name = devices
            .iter()
            .find(|device| device.id == notification.id.device_id)
            .map(|device| device.name.as_str())
            .unwrap_or_else(|| notification.id.device_id.as_str());
        println!(
            "{}  {}  {}",
            device_name, notification.app_name, notification.title
        );
    }
}

fn print_message(payload: ServerPayload) {
    match payload {
        ServerPayload::CallUpdated { call } => {
            println!("call state: {} {:?}", call.device_id, call.phase)
        }
        ServerPayload::CallRemoved { device_id } => println!("call state unavailable: {device_id}"),
        ServerPayload::CallCommandResult { result } => println!(
            "call command result: {} {:?} accepted={} failure={:?}",
            result.device_id, result.action, result.accepted, result.failure
        ),
        ServerPayload::DeviceCommandResult { result } => println!(
            "device command result: {} {:?} accepted={} failure={:?}",
            result.device_id, result.action, result.accepted, result.failure
        ),
        ServerPayload::CallQueued { request_id } => println!("call command queued: {request_id}"),
        ServerPayload::Calls { .. } | ServerPayload::CallAudio { .. } => {}
        ServerPayload::DeviceAdded { device } => {
            println!("device added: {}", describe_device(&device));
        }
        ServerPayload::DeviceUpdated { device } => {
            println!("device updated: {}", describe_device(&device));
        }
        ServerPayload::DeviceRemoved { device_id } => {
            println!("device removed: {device_id}");
        }
        ServerPayload::NotificationAdded { notification } => {
            println!(
                "notification added: {} — {}",
                notification.app_name, notification.title
            );
        }
        ServerPayload::NotificationUpdated { notification } => {
            println!(
                "notification updated: {} — {}",
                notification.app_name, notification.title
            );
        }
        ServerPayload::NotificationRemoved { notification_id } => {
            println!("notification removed: {notification_id}");
        }
        ServerPayload::MediaAdded { media_session } => {
            println!("media added: {}", describe_media(&media_session));
        }
        ServerPayload::MediaUpdated { media_session } => {
            println!("media updated: {}", describe_media(&media_session));
        }
        ServerPayload::MediaRemoved { media_session_id } => {
            println!("media removed: {}", media_selector(&media_session_id));
        }
        ServerPayload::ShareReceived { share } => {
            let kind = match share.resource {
                SharedResource::File { .. } => "local file available",
                SharedResource::Url { .. } => "URL",
            };
            println!("share received: {kind} from {}", share.device_id);
        }
        ServerPayload::ShareProgress { progress } => println!(
            "share progress: transfer {} to {} sent {}/{} bytes",
            progress.transfer_id, progress.device_id, progress.bytes_sent, progress.total_bytes
        ),
        ServerPayload::ShareResult { result } => {
            println!(
                "share {}: transfer {} to {}{}",
                match result.status {
                    handover_core::ShareStatus::Completed => "completed",
                    handover_core::ShareStatus::Failed => "failed",
                },
                result.transfer_id,
                result.device_id,
                result
                    .reason
                    .map_or_else(String::new, |reason| format!(" ({reason:?})"))
            );
        }
        ServerPayload::ShareCancelled { cancelled } => {
            println!(
                "share {}",
                if cancelled {
                    "cancelled before streaming started"
                } else {
                    "was already sent or is unknown"
                }
            );
        }
        ServerPayload::CustomCommands { commands } => {
            println!("NAME  ARGV");
            for entry in commands {
                println!("{}  {}", entry.name, entry.argv.join(" "));
            }
        }
        ServerPayload::CustomResult { result } => {
            println!(
                "custom command {}: accepted={} exit={:?} failure={:?}",
                result.name, result.accepted, result.exit_code, result.failure
            );
        }
        ServerPayload::ClipboardMirror { enabled } => {
            println!(
                "background clipboard mirroring: {}",
                if enabled { "on" } else { "off" }
            );
        }
        ServerPayload::Snapshot {
            devices,
            notifications,
            media_sessions,
            ..
        } => {
            println!(
                "state resynchronized: {} device(s), {} notification(s), {} media session(s)",
                devices.len(),
                notifications.len(),
                media_sessions.len()
            );
        }
        ServerPayload::AccountAdded { account } => {
            println!("messaging account added: {}", account.id);
        }
        ServerPayload::AccountUpdated { account } => {
            println!(
                "messaging account updated: {} connected={} authenticated={}",
                account.id, account.connected, account.authenticated
            );
        }
        ServerPayload::AccountRemoved { account_id } => {
            println!("messaging account removed: {account_id}");
        }
        ServerPayload::ConversationAdded { conversation } => {
            println!("conversation added: {}", conversation.id);
        }
        ServerPayload::ConversationUpdated { conversation } => {
            println!("conversation updated: {}", conversation.id);
        }
        ServerPayload::ConversationRemoved { conversation_id } => {
            println!("conversation removed: {conversation_id}");
        }
        ServerPayload::MessageAdded { message } => {
            println!("message added: {}", message.id);
        }
        ServerPayload::MessageUpdated { message } => {
            println!("message updated: {}", message.id);
        }
        ServerPayload::MessageRemoved { message_id } => {
            println!("message removed: {message_id}");
        }
        ServerPayload::MessageStatus { update } => {
            println!("message status: {} {:?}", update.message_id, update.status);
        }
        ServerPayload::Typing { state } => {
            println!(
                "typing in {}: {}",
                state.conversation_id,
                state.participant_ids.join(", ")
            );
        }
        ServerPayload::ReadState { state } => {
            println!(
                "read state for {}: unread={}",
                state.conversation_id, state.unread
            );
        }
        ServerPayload::Pairing { account_id, prompt } => {
            println!("pairing for {account_id}: {prompt}");
        }
        ServerPayload::Error { code, message } => {
            eprintln!("daemon error ({code:?}): {message}");
        }
        ServerPayload::Hello { .. }
        | ServerPayload::Devices { .. }
        | ServerPayload::Notifications { .. }
        | ServerPayload::Contacts { .. }
        | ServerPayload::Media { .. }
        | ServerPayload::Accounts { .. }
        | ServerPayload::Conversations { .. }
        | ServerPayload::History { .. }
        | ServerPayload::TypingStates { .. }
        | ServerPayload::ReadStates { .. }
        | ServerPayload::CommandCompleted { .. }
        | ServerPayload::ShareAccepted { .. }
        | ServerPayload::MediaAccepted { .. }
        | ServerPayload::MessageAccepted { .. }
        | ServerPayload::ConversationAccepted { .. }
        | ServerPayload::AccountAccepted { .. }
        | ServerPayload::Subscribed { .. }
        | ServerPayload::NativePeers { .. }
        | ServerPayload::NativePending { .. }
        | ServerPayload::ClipboardHistory { .. }
        | ServerPayload::NativeAccepted => {}
    }
}

fn print_media_table(devices: &[Device], sessions: &[MediaSession]) {
    println!("SESSION  DEVICE  APP  STATE  TITLE  ARTIST");
    for session in sessions {
        let device_name = devices
            .iter()
            .find(|device| device.id == session.id.device_id)
            .map(|device| device.name.as_str())
            .unwrap_or_else(|| session.id.device_id.as_str());
        println!(
            "{}  {}  {}  {}  {}  {}",
            media_selector(&session.id),
            device_name,
            session.application,
            playback_label(session),
            session.title.as_deref().unwrap_or("-"),
            session.artist.as_deref().unwrap_or("-")
        );
    }
}

fn playback_label(session: &MediaSession) -> &'static str {
    match session.playback {
        handover_core::PlaybackState::Playing => "playing",
        handover_core::PlaybackState::Paused => "paused",
        handover_core::PlaybackState::Stopped => "stopped",
        handover_core::PlaybackState::Unknown => "unknown",
    }
}

fn media_selector(id: &MediaSessionId) -> String {
    format!("{}:{}", id.device_id, id.player_id)
}

fn select_media_session(
    sessions: &[MediaSession],
    selector: &str,
) -> Result<MediaSessionId, CliError> {
    if let Some(session) = sessions
        .iter()
        .find(|session| media_selector(&session.id) == selector)
    {
        return Ok(session.id.clone());
    }

    let matches: Vec<_> = sessions
        .iter()
        .filter(|session| session.application == selector)
        .collect();
    match matches.as_slice() {
        [session] => Ok(session.id.clone()),
        [] => Err(CliError::MediaSelection(format!(
            "no media session identified by {selector:?}"
        ))),
        _ => Err(CliError::MediaSelection(format!(
            "multiple media sessions use application {selector:?}; use a session ID"
        ))),
    }
}

fn describe_media(session: &MediaSession) -> String {
    format!(
        "{} {} state={} title={}",
        media_selector(&session.id),
        session.application,
        playback_label(session),
        session.title.as_deref().unwrap_or("-")
    )
}

fn describe_device(device: &Device) -> String {
    let battery = device
        .battery
        .map(|battery| format!("{}% charging={}", battery.percentage(), battery.charging))
        .unwrap_or_else(|| "unavailable".into());
    format!(
        "{} id={} connected={} paired={} battery={}",
        device.name, device.id, device.connected, device.paired, battery
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use handover_core::{BatteryState, Capability, DeviceId, MediaSession, PlaybackState};

    use super::*;

    #[test]
    fn describes_normalized_device() {
        let device = Device {
            id: DeviceId::new("phone-123"),
            name: "Phone".into(),
            connected: true,
            paired: true,
            battery: Some(BatteryState::new(55, false).expect("valid battery")),
            connectivity: None,
            capabilities: BTreeSet::from([Capability::Battery]),
        };

        assert_eq!(
            describe_device(&device),
            "Phone id=phone-123 connected=true paired=true battery=55% charging=false"
        );
    }

    #[test]
    fn selects_id_or_unique_exact_name_without_guessing() {
        let mut first = Device {
            id: DeviceId::new("phone-a"),
            name: "Phone".into(),
            connected: true,
            paired: true,
            battery: None,
            connectivity: None,
            capabilities: BTreeSet::new(),
        };
        let second = Device {
            id: DeviceId::new("phone-b"),
            name: "Tablet".into(),
            ..first.clone()
        };
        assert_eq!(
            select_device(&[first.clone(), second.clone()], "Tablet").expect("unique name"),
            second.id
        );
        assert_eq!(
            select_device(&[first.clone(), second], "phone-a").expect("ID"),
            first.id
        );
        first.id = DeviceId::new("phone-c");
        assert!(matches!(
            select_device(&[first.clone(), Device { id: DeviceId::new("phone-a"), ..first }], "Phone"),
            Err(CliError::DeviceSelection(message)) if message.contains("multiple")
        ));
        assert!(matches!(
            select_device(&[], "missing"),
            Err(CliError::DeviceSelection(message)) if message.contains("no device")
        ));
    }

    #[test]
    fn native_commands_select_peers_by_name_or_id() {
        let peers = vec![
            handover_ipc::NativePeer {
                id: "peer-a".into(),
                name: "SM-S948U1".into(),
                fingerprint: "peer-a".into(),
            },
            handover_ipc::NativePeer {
                id: "peer-b".into(),
                name: "Tablet".into(),
                fingerprint: "peer-b".into(),
            },
        ];
        assert_eq!(
            select_native_peer_from_slice(&peers, "SM-S948U1").expect("name"),
            "peer-a"
        );
        assert_eq!(
            select_native_peer_from_slice(&peers, "peer-b").expect("ID"),
            "peer-b"
        );
    }

    fn media_session(device_id: &str, player_id: &str, application: &str) -> MediaSession {
        MediaSession {
            id: MediaSessionId::new(DeviceId::new(device_id), player_id),
            application: application.into(),
            title: Some("Song".into()),
            artist: Some("Artist".into()),
            album: None,
            playback: PlaybackState::Playing,
            position_ms: None,
            duration_ms: None,
            volume_percent: None,
            controls: Default::default(),
        }
    }

    #[test]
    fn selects_media_by_exact_scoped_id_or_unique_application() {
        let first = media_session("phone-a", "player-1", "Spotify");
        let second = media_session("phone-b", "player-2", "Music");
        assert_eq!(
            select_media_session(&[first.clone(), second.clone()], "phone-a:player-1")
                .expect("scoped ID"),
            first.id
        );
        assert_eq!(
            select_media_session(&[first.clone(), second.clone()], "Music").expect("unique app"),
            second.id
        );
    }

    #[test]
    fn rejects_ambiguous_or_unknown_media_selector() {
        let first = media_session("phone-a", "player-1", "Spotify");
        let second = media_session("phone-b", "player-2", "Spotify");
        assert!(matches!(
            select_media_session(&[first.clone(), second], "Spotify"),
            Err(CliError::MediaSelection(message)) if message.contains("multiple")
        ));
        assert!(matches!(
            select_media_session(&[first], "missing"),
            Err(CliError::MediaSelection(message)) if message.contains("no media session")
        ));
    }

    #[test]
    fn formats_media_state_without_optional_metadata() {
        let mut session = media_session("phone-a", "player-1", "Spotify");
        session.title = None;
        session.playback = PlaybackState::Paused;
        assert_eq!(
            describe_media(&session),
            "phone-a:player-1 Spotify state=paused title=-"
        );
    }
}

#[cfg(test)]
mod messaging_tests {
    use super::*;

    #[tokio::test]
    async fn bundle_reads_stop_past_the_limit() {
        let path = std::env::temp_dir().join("handover-cli-bundle-limit-test");
        let oversized = vec![b'x'; MAX_RAW_BUNDLE_BYTES as usize + 100];
        tokio::fs::write(&path, &oversized)
            .await
            .expect("fixture writes");
        let bundle = read_bundle(Some(path.clone())).await.expect("reads");
        let _ = tokio::fs::remove_file(&path).await;
        // Oversize input is detected (read past the cap) without
        // ever buffering the whole thing.
        assert!(bundle.len() as u64 > MAX_RAW_BUNDLE_BYTES);
        assert!(bundle.len() <= MAX_RAW_BUNDLE_BYTES as usize + 1);
    }

    #[test]
    fn raw_cap_encodes_within_daemon_cap() {
        // 192 KiB raw is exactly 256 KiB encoded: what the CLI accepts,
        // the daemon admits.
        assert_eq!(MAX_RAW_BUNDLE_BYTES / 3 * 4, 256 * 1024);
    }

    #[test]
    fn base64_encode_matches_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(
            base64_encode(b" OPAQUE-BUNDLE:42/+"),
            "IE9QQVFVRS1CVU5ETEU6NDIvKw=="
        );
    }

    #[test]
    fn conversation_selector_prefers_longest_account_prefix() {
        // Account ids contain colons (`gmessages:default`), so the
        // qualified form `gmessages:default:thread-1` must match the full
        // account prefix, not the first colon.
        let selector = "gmessages:default:thread-1";
        let accounts = ["gmessages", "gmessages:default"];
        let mut best: Option<&str> = None;
        for account in accounts {
            let prefix = format!("{account}:");
            if let Some(rest) = selector.strip_prefix(&prefix) {
                if best.is_none_or(|known: &str| known.len() > rest.len()) {
                    best = Some(rest);
                }
            }
        }
        assert_eq!(best, Some("thread-1"));
        assert_eq!("thread-1".split_once(':'), None);
    }
}
