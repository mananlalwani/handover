
use builtin;
use str;

set edit:completion:arg-completer[handoverctl] = {|@words|
    fn spaces {|n|
        builtin:repeat $n ' ' | str:join ''
    }
    fn cand {|text desc|
        edit:complex-candidate $text &display=$text' '(spaces (- 14 (wcswidth $text)))$desc
    }
    var command = 'handoverctl'
    for word $words[1..-1] {
        if (str:has-prefix $word '-') {
            break
        }
        set command = $command';'$word
    }
    var completions = [
        &'handoverctl'= {
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
            cand -V 'Print version'
            cand --version 'Print version'
            cand devices 'List native devices known to handoverd'
            cand native 'Inspect and manage native Android pairing and device commands'
            cand notifications 'List active remote notifications from the daemon snapshot'
            cand contacts 'List or request an on-demand native contacts snapshot'
            cand media 'List media sessions, or send one playback command'
            cand monitor 'Print live normalized device and notification changes'
            cand send-url 'Send a URL to one paired device'
            cand notify 'Send a notification to one native phone'
            cand clipboard 'Set the clipboard on one native phone'
            cand send-file 'Send one local file to a paired device'
            cand screensaver 'Control the desktop idle inhibitor: inhibit, release, or follow'
            cand clipboard-mirror 'Get or set Linux-to-phone background clipboard mirroring (off by default)'
            cand clipboard-history 'List, pin, copy, and clear phone-to-Linux clipboard history'
            cand cancel-share 'Cancel a queued native share that has not started streaming'
            cand custom 'List or run allowlisted desktop commands'
            cand calls 'Print current normalized call state for one device'
            cand messages 'Inspect and use messaging accounts and conversations'
            cand help 'Print this message or the help of the given subcommand(s)'
        }
        &'handoverctl;devices'= {
            cand --include-compatibility 'Include devices supplied by the optional KDE Connect backend'
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'handoverctl;native'= {
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
            cand peers 'List trusted native peers (id, name, certificate fingerprint)'
            cand pending 'List pending pairing requests (id, name, eight-digit code)'
            cand pair 'Approve a pending pairing request after comparing codes'
            cand unpair 'Revoke a trusted native peer'
            cand ping 'Send a user-visible liveness ping to a native device name or ID'
            cand ring 'Ring and vibrate a native device selected by name or ID'
            cand lock 'Lock a native phone when device-admin access is enabled'
            cand keep-awake 'Ask a native phone to hold its wake lock (or release it with --release)'
            cand tethering 'Ask a native phone to open its tethering settings screen'
            cand filesystem-list 'Ask the phone for a bounded directory listing. Results arrive through `monitor` because the native transfer is asynchronous'
            cand call 'Send a call control action to a native phone'
            cand help 'Print this message or the help of the given subcommand(s)'
        }
        &'handoverctl;native;peers'= {
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'handoverctl;native;pending'= {
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'handoverctl;native;pair'= {
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'handoverctl;native;unpair'= {
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'handoverctl;native;ping'= {
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'handoverctl;native;ring'= {
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'handoverctl;native;lock'= {
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'handoverctl;native;keep-awake'= {
            cand --release 'Release the phone wake lock instead of holding it'
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'handoverctl;native;tethering'= {
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'handoverctl;native;filesystem-list'= {
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'handoverctl;native;call'= {
            cand --confirm 'Explicitly authorize placing a real phone call (required for place)'
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'handoverctl;native;help'= {
            cand peers 'List trusted native peers (id, name, certificate fingerprint)'
            cand pending 'List pending pairing requests (id, name, eight-digit code)'
            cand pair 'Approve a pending pairing request after comparing codes'
            cand unpair 'Revoke a trusted native peer'
            cand ping 'Send a user-visible liveness ping to a native device name or ID'
            cand ring 'Ring and vibrate a native device selected by name or ID'
            cand lock 'Lock a native phone when device-admin access is enabled'
            cand keep-awake 'Ask a native phone to hold its wake lock (or release it with --release)'
            cand tethering 'Ask a native phone to open its tethering settings screen'
            cand filesystem-list 'Ask the phone for a bounded directory listing. Results arrive through `monitor` because the native transfer is asynchronous'
            cand call 'Send a call control action to a native phone'
            cand help 'Print this message or the help of the given subcommand(s)'
        }
        &'handoverctl;native;help;peers'= {
        }
        &'handoverctl;native;help;pending'= {
        }
        &'handoverctl;native;help;pair'= {
        }
        &'handoverctl;native;help;unpair'= {
        }
        &'handoverctl;native;help;ping'= {
        }
        &'handoverctl;native;help;ring'= {
        }
        &'handoverctl;native;help;lock'= {
        }
        &'handoverctl;native;help;keep-awake'= {
        }
        &'handoverctl;native;help;tethering'= {
        }
        &'handoverctl;native;help;filesystem-list'= {
        }
        &'handoverctl;native;help;call'= {
        }
        &'handoverctl;native;help;help'= {
        }
        &'handoverctl;notifications'= {
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'handoverctl;contacts'= {
            cand -h 'Print help'
            cand --help 'Print help'
            cand list 'List the latest contacts snapshot held by handoverd'
            cand sync 'Ask one native phone to send a fresh contacts snapshot'
            cand help 'Print this message or the help of the given subcommand(s)'
        }
        &'handoverctl;contacts;list'= {
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'handoverctl;contacts;sync'= {
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'handoverctl;contacts;help'= {
            cand list 'List the latest contacts snapshot held by handoverd'
            cand sync 'Ask one native phone to send a fresh contacts snapshot'
            cand help 'Print this message or the help of the given subcommand(s)'
        }
        &'handoverctl;contacts;help;list'= {
        }
        &'handoverctl;contacts;help;sync'= {
        }
        &'handoverctl;contacts;help;help'= {
        }
        &'handoverctl;media'= {
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
            cand play 'Start playback on one session'
            cand pause 'Pause playback on one session'
            cand play-pause 'Toggle playback on one session'
            cand next 'Skip to the next item on one session'
            cand previous 'Return to the previous item on one session'
            cand help 'Print this message or the help of the given subcommand(s)'
        }
        &'handoverctl;media;play'= {
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'handoverctl;media;pause'= {
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'handoverctl;media;play-pause'= {
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'handoverctl;media;next'= {
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'handoverctl;media;previous'= {
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'handoverctl;media;help'= {
            cand play 'Start playback on one session'
            cand pause 'Pause playback on one session'
            cand play-pause 'Toggle playback on one session'
            cand next 'Skip to the next item on one session'
            cand previous 'Return to the previous item on one session'
            cand help 'Print this message or the help of the given subcommand(s)'
        }
        &'handoverctl;media;help;play'= {
        }
        &'handoverctl;media;help;pause'= {
        }
        &'handoverctl;media;help;play-pause'= {
        }
        &'handoverctl;media;help;next'= {
        }
        &'handoverctl;media;help;previous'= {
        }
        &'handoverctl;media;help;help'= {
        }
        &'handoverctl;monitor'= {
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'handoverctl;send-url'= {
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'handoverctl;notify'= {
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'handoverctl;clipboard'= {
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'handoverctl;send-file'= {
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'handoverctl;screensaver'= {
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'handoverctl;clipboard-mirror'= {
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'handoverctl;clipboard-history'= {
            cand -h 'Print help'
            cand --help 'Print help'
            cand list 'List recent and pinned entries (text previews only)'
            cand save 'Save a new pinned string'
            cand pin 'Pin an existing entry by id'
            cand unpin 'Unpin an existing entry by id'
            cand copy 'Copy an entry into the Linux clipboard'
            cand clear 'Remove recent entries, retaining pins unless --all is passed'
            cand help 'Print this message or the help of the given subcommand(s)'
        }
        &'handoverctl;clipboard-history;list'= {
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'handoverctl;clipboard-history;save'= {
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'handoverctl;clipboard-history;pin'= {
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'handoverctl;clipboard-history;unpin'= {
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'handoverctl;clipboard-history;copy'= {
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'handoverctl;clipboard-history;clear'= {
            cand --all 'Also remove pinned entries'
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'handoverctl;clipboard-history;help'= {
            cand list 'List recent and pinned entries (text previews only)'
            cand save 'Save a new pinned string'
            cand pin 'Pin an existing entry by id'
            cand unpin 'Unpin an existing entry by id'
            cand copy 'Copy an entry into the Linux clipboard'
            cand clear 'Remove recent entries, retaining pins unless --all is passed'
            cand help 'Print this message or the help of the given subcommand(s)'
        }
        &'handoverctl;clipboard-history;help;list'= {
        }
        &'handoverctl;clipboard-history;help;save'= {
        }
        &'handoverctl;clipboard-history;help;pin'= {
        }
        &'handoverctl;clipboard-history;help;unpin'= {
        }
        &'handoverctl;clipboard-history;help;copy'= {
        }
        &'handoverctl;clipboard-history;help;clear'= {
        }
        &'handoverctl;clipboard-history;help;help'= {
        }
        &'handoverctl;cancel-share'= {
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'handoverctl;custom'= {
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
            cand list 'List allowlisted desktop commands with their fixed argv'
            cand run 'Run one allowlisted desktop command by name'
            cand help 'Print this message or the help of the given subcommand(s)'
        }
        &'handoverctl;custom;list'= {
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'handoverctl;custom;run'= {
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'handoverctl;custom;help'= {
            cand list 'List allowlisted desktop commands with their fixed argv'
            cand run 'Run one allowlisted desktop command by name'
            cand help 'Print this message or the help of the given subcommand(s)'
        }
        &'handoverctl;custom;help;list'= {
        }
        &'handoverctl;custom;help;run'= {
        }
        &'handoverctl;custom;help;help'= {
        }
        &'handoverctl;calls'= {
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'handoverctl;messages'= {
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
            cand accounts 'List messaging accounts with connection state'
            cand conversations 'List conversations for one account'
            cand history 'Show message history for one conversation (ACCOUNT:THREAD or THREAD)'
            cand send 'Send a text message (accepted, not delivered)'
            cand send-file 'Send a file attachment with an optional caption'
            cand reply 'Reply to a message (ACCOUNT:THREAD:MESSAGE or THREAD:MESSAGE ...)'
            cand react 'Add a reaction to a message'
            cand unreact 'Remove a reaction from a message'
            cand read 'Mark a conversation read (optionally up to one message)'
            cand typing 'Send a typing-start ping (no typing-stop exists upstream)'
            cand delete 'Delete one own message'
            cand open 'Open or create a conversation with addresses (phone numbers/emails)'
            cand login 'Log in: read a credential bundle from a file or stdin, never argv'
            cand logout 'Log out and revoke helper access'
            cand sync 'Ask the helper to re-emit authoritative state for one account'
            cand help 'Print this message or the help of the given subcommand(s)'
        }
        &'handoverctl;messages;accounts'= {
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'handoverctl;messages;conversations'= {
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'handoverctl;messages;history'= {
            cand --limit 'Maximum messages to show'
            cand --cursor 'Cursor printed by a previous history page'
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'handoverctl;messages;send'= {
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'handoverctl;messages;send-file'= {
            cand --caption 'Optional caption sent with the attachment'
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'handoverctl;messages;reply'= {
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'handoverctl;messages;react'= {
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'handoverctl;messages;unreact'= {
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'handoverctl;messages;read'= {
            cand --message 'Message selector marking the read point'
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'handoverctl;messages;typing'= {
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'handoverctl;messages;delete'= {
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'handoverctl;messages;open'= {
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'handoverctl;messages;login'= {
            cand --from-file 'Read the credential bundle from PATH instead of stdin'
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'handoverctl;messages;logout'= {
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'handoverctl;messages;sync'= {
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'handoverctl;messages;help'= {
            cand accounts 'List messaging accounts with connection state'
            cand conversations 'List conversations for one account'
            cand history 'Show message history for one conversation (ACCOUNT:THREAD or THREAD)'
            cand send 'Send a text message (accepted, not delivered)'
            cand send-file 'Send a file attachment with an optional caption'
            cand reply 'Reply to a message (ACCOUNT:THREAD:MESSAGE or THREAD:MESSAGE ...)'
            cand react 'Add a reaction to a message'
            cand unreact 'Remove a reaction from a message'
            cand read 'Mark a conversation read (optionally up to one message)'
            cand typing 'Send a typing-start ping (no typing-stop exists upstream)'
            cand delete 'Delete one own message'
            cand open 'Open or create a conversation with addresses (phone numbers/emails)'
            cand login 'Log in: read a credential bundle from a file or stdin, never argv'
            cand logout 'Log out and revoke helper access'
            cand sync 'Ask the helper to re-emit authoritative state for one account'
            cand help 'Print this message or the help of the given subcommand(s)'
        }
        &'handoverctl;messages;help;accounts'= {
        }
        &'handoverctl;messages;help;conversations'= {
        }
        &'handoverctl;messages;help;history'= {
        }
        &'handoverctl;messages;help;send'= {
        }
        &'handoverctl;messages;help;send-file'= {
        }
        &'handoverctl;messages;help;reply'= {
        }
        &'handoverctl;messages;help;react'= {
        }
        &'handoverctl;messages;help;unreact'= {
        }
        &'handoverctl;messages;help;read'= {
        }
        &'handoverctl;messages;help;typing'= {
        }
        &'handoverctl;messages;help;delete'= {
        }
        &'handoverctl;messages;help;open'= {
        }
        &'handoverctl;messages;help;login'= {
        }
        &'handoverctl;messages;help;logout'= {
        }
        &'handoverctl;messages;help;sync'= {
        }
        &'handoverctl;messages;help;help'= {
        }
        &'handoverctl;help'= {
            cand devices 'List native devices known to handoverd'
            cand native 'Inspect and manage native Android pairing and device commands'
            cand notifications 'List active remote notifications from the daemon snapshot'
            cand contacts 'List or request an on-demand native contacts snapshot'
            cand media 'List media sessions, or send one playback command'
            cand monitor 'Print live normalized device and notification changes'
            cand send-url 'Send a URL to one paired device'
            cand notify 'Send a notification to one native phone'
            cand clipboard 'Set the clipboard on one native phone'
            cand send-file 'Send one local file to a paired device'
            cand screensaver 'Control the desktop idle inhibitor: inhibit, release, or follow'
            cand clipboard-mirror 'Get or set Linux-to-phone background clipboard mirroring (off by default)'
            cand clipboard-history 'List, pin, copy, and clear phone-to-Linux clipboard history'
            cand cancel-share 'Cancel a queued native share that has not started streaming'
            cand custom 'List or run allowlisted desktop commands'
            cand calls 'Print current normalized call state for one device'
            cand messages 'Inspect and use messaging accounts and conversations'
            cand help 'Print this message or the help of the given subcommand(s)'
        }
        &'handoverctl;help;devices'= {
        }
        &'handoverctl;help;native'= {
            cand peers 'List trusted native peers (id, name, certificate fingerprint)'
            cand pending 'List pending pairing requests (id, name, eight-digit code)'
            cand pair 'Approve a pending pairing request after comparing codes'
            cand unpair 'Revoke a trusted native peer'
            cand ping 'Send a user-visible liveness ping to a native device name or ID'
            cand ring 'Ring and vibrate a native device selected by name or ID'
            cand lock 'Lock a native phone when device-admin access is enabled'
            cand keep-awake 'Ask a native phone to hold its wake lock (or release it with --release)'
            cand tethering 'Ask a native phone to open its tethering settings screen'
            cand filesystem-list 'Ask the phone for a bounded directory listing. Results arrive through `monitor` because the native transfer is asynchronous'
            cand call 'Send a call control action to a native phone'
        }
        &'handoverctl;help;native;peers'= {
        }
        &'handoverctl;help;native;pending'= {
        }
        &'handoverctl;help;native;pair'= {
        }
        &'handoverctl;help;native;unpair'= {
        }
        &'handoverctl;help;native;ping'= {
        }
        &'handoverctl;help;native;ring'= {
        }
        &'handoverctl;help;native;lock'= {
        }
        &'handoverctl;help;native;keep-awake'= {
        }
        &'handoverctl;help;native;tethering'= {
        }
        &'handoverctl;help;native;filesystem-list'= {
        }
        &'handoverctl;help;native;call'= {
        }
        &'handoverctl;help;notifications'= {
        }
        &'handoverctl;help;contacts'= {
            cand list 'List the latest contacts snapshot held by handoverd'
            cand sync 'Ask one native phone to send a fresh contacts snapshot'
        }
        &'handoverctl;help;contacts;list'= {
        }
        &'handoverctl;help;contacts;sync'= {
        }
        &'handoverctl;help;media'= {
            cand play 'Start playback on one session'
            cand pause 'Pause playback on one session'
            cand play-pause 'Toggle playback on one session'
            cand next 'Skip to the next item on one session'
            cand previous 'Return to the previous item on one session'
        }
        &'handoverctl;help;media;play'= {
        }
        &'handoverctl;help;media;pause'= {
        }
        &'handoverctl;help;media;play-pause'= {
        }
        &'handoverctl;help;media;next'= {
        }
        &'handoverctl;help;media;previous'= {
        }
        &'handoverctl;help;monitor'= {
        }
        &'handoverctl;help;send-url'= {
        }
        &'handoverctl;help;notify'= {
        }
        &'handoverctl;help;clipboard'= {
        }
        &'handoverctl;help;send-file'= {
        }
        &'handoverctl;help;screensaver'= {
        }
        &'handoverctl;help;clipboard-mirror'= {
        }
        &'handoverctl;help;clipboard-history'= {
            cand list 'List recent and pinned entries (text previews only)'
            cand save 'Save a new pinned string'
            cand pin 'Pin an existing entry by id'
            cand unpin 'Unpin an existing entry by id'
            cand copy 'Copy an entry into the Linux clipboard'
            cand clear 'Remove recent entries, retaining pins unless --all is passed'
        }
        &'handoverctl;help;clipboard-history;list'= {
        }
        &'handoverctl;help;clipboard-history;save'= {
        }
        &'handoverctl;help;clipboard-history;pin'= {
        }
        &'handoverctl;help;clipboard-history;unpin'= {
        }
        &'handoverctl;help;clipboard-history;copy'= {
        }
        &'handoverctl;help;clipboard-history;clear'= {
        }
        &'handoverctl;help;cancel-share'= {
        }
        &'handoverctl;help;custom'= {
            cand list 'List allowlisted desktop commands with their fixed argv'
            cand run 'Run one allowlisted desktop command by name'
        }
        &'handoverctl;help;custom;list'= {
        }
        &'handoverctl;help;custom;run'= {
        }
        &'handoverctl;help;calls'= {
        }
        &'handoverctl;help;messages'= {
            cand accounts 'List messaging accounts with connection state'
            cand conversations 'List conversations for one account'
            cand history 'Show message history for one conversation (ACCOUNT:THREAD or THREAD)'
            cand send 'Send a text message (accepted, not delivered)'
            cand send-file 'Send a file attachment with an optional caption'
            cand reply 'Reply to a message (ACCOUNT:THREAD:MESSAGE or THREAD:MESSAGE ...)'
            cand react 'Add a reaction to a message'
            cand unreact 'Remove a reaction from a message'
            cand read 'Mark a conversation read (optionally up to one message)'
            cand typing 'Send a typing-start ping (no typing-stop exists upstream)'
            cand delete 'Delete one own message'
            cand open 'Open or create a conversation with addresses (phone numbers/emails)'
            cand login 'Log in: read a credential bundle from a file or stdin, never argv'
            cand logout 'Log out and revoke helper access'
            cand sync 'Ask the helper to re-emit authoritative state for one account'
        }
        &'handoverctl;help;messages;accounts'= {
        }
        &'handoverctl;help;messages;conversations'= {
        }
        &'handoverctl;help;messages;history'= {
        }
        &'handoverctl;help;messages;send'= {
        }
        &'handoverctl;help;messages;send-file'= {
        }
        &'handoverctl;help;messages;reply'= {
        }
        &'handoverctl;help;messages;react'= {
        }
        &'handoverctl;help;messages;unreact'= {
        }
        &'handoverctl;help;messages;read'= {
        }
        &'handoverctl;help;messages;typing'= {
        }
        &'handoverctl;help;messages;delete'= {
        }
        &'handoverctl;help;messages;open'= {
        }
        &'handoverctl;help;messages;login'= {
        }
        &'handoverctl;help;messages;logout'= {
        }
        &'handoverctl;help;messages;sync'= {
        }
        &'handoverctl;help;help'= {
        }
    ]
    $completions[$command]
}
