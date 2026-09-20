
using namespace System.Management.Automation
using namespace System.Management.Automation.Language

Register-ArgumentCompleter -Native -CommandName 'handoverctl' -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)

    $commandElements = $commandAst.CommandElements
    $command = @(
        'handoverctl'
        for ($i = 1; $i -lt $commandElements.Count; $i++) {
            $element = $commandElements[$i]
            if ($element -isnot [StringConstantExpressionAst] -or
                $element.StringConstantType -ne [StringConstantType]::BareWord -or
                $element.Value.StartsWith('-') -or
                $element.Value -eq $wordToComplete) {
                break
        }
        $element.Value
    }) -join ';'

    $completions = @(switch ($command) {
        'handoverctl' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('-V', '-V ', [CompletionResultType]::ParameterName, 'Print version')
            [CompletionResult]::new('--version', '--version', [CompletionResultType]::ParameterName, 'Print version')
            [CompletionResult]::new('devices', 'devices', [CompletionResultType]::ParameterValue, 'List devices known to handoverd')
            [CompletionResult]::new('native', 'native', [CompletionResultType]::ParameterValue, 'Inspect and manage native Android pairing and device commands')
            [CompletionResult]::new('notifications', 'notifications', [CompletionResultType]::ParameterValue, 'List active remote notifications from the daemon snapshot')
            [CompletionResult]::new('contacts', 'contacts', [CompletionResultType]::ParameterValue, 'List or request an on-demand native contacts snapshot')
            [CompletionResult]::new('media', 'media', [CompletionResultType]::ParameterValue, 'List media sessions, or send one playback command')
            [CompletionResult]::new('monitor', 'monitor', [CompletionResultType]::ParameterValue, 'Print live normalized device and notification changes')
            [CompletionResult]::new('send-url', 'send-url', [CompletionResultType]::ParameterValue, 'Send a URL to one paired device')
            [CompletionResult]::new('notify', 'notify', [CompletionResultType]::ParameterValue, 'Send a notification to one native phone')
            [CompletionResult]::new('clipboard', 'clipboard', [CompletionResultType]::ParameterValue, 'Set the clipboard on one native phone')
            [CompletionResult]::new('send-file', 'send-file', [CompletionResultType]::ParameterValue, 'Send one local file to a paired device')
            [CompletionResult]::new('screensaver', 'screensaver', [CompletionResultType]::ParameterValue, 'Control the desktop idle inhibitor: inhibit, release, or follow')
            [CompletionResult]::new('clipboard-mirror', 'clipboard-mirror', [CompletionResultType]::ParameterValue, 'Get or set Linux-to-phone background clipboard mirroring (off by default)')
            [CompletionResult]::new('clipboard-history', 'clipboard-history', [CompletionResultType]::ParameterValue, 'List, pin, copy, and clear phone-to-Linux clipboard history')
            [CompletionResult]::new('cancel-share', 'cancel-share', [CompletionResultType]::ParameterValue, 'Cancel a queued native share that has not started streaming')
            [CompletionResult]::new('custom', 'custom', [CompletionResultType]::ParameterValue, 'List or run allowlisted desktop commands')
            [CompletionResult]::new('calls', 'calls', [CompletionResultType]::ParameterValue, 'Print current normalized call state for one device')
            [CompletionResult]::new('messages', 'messages', [CompletionResultType]::ParameterValue, 'Inspect and use messaging accounts and conversations')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'handoverctl;devices' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'handoverctl;native' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('peers', 'peers', [CompletionResultType]::ParameterValue, 'List trusted native peers (id, name, certificate fingerprint)')
            [CompletionResult]::new('pending', 'pending', [CompletionResultType]::ParameterValue, 'List pending pairing requests (id, name, eight-digit code)')
            [CompletionResult]::new('pair', 'pair', [CompletionResultType]::ParameterValue, 'Approve a pending pairing request after comparing codes')
            [CompletionResult]::new('unpair', 'unpair', [CompletionResultType]::ParameterValue, 'Revoke a trusted native peer')
            [CompletionResult]::new('ping', 'ping', [CompletionResultType]::ParameterValue, 'Send a user-visible liveness ping to a native device name or ID')
            [CompletionResult]::new('ring', 'ring', [CompletionResultType]::ParameterValue, 'Ring and vibrate a native device selected by name or ID')
            [CompletionResult]::new('lock', 'lock', [CompletionResultType]::ParameterValue, 'Lock a native phone when device-admin access is enabled')
            [CompletionResult]::new('keep-awake', 'keep-awake', [CompletionResultType]::ParameterValue, 'Ask a native phone to hold its wake lock (or release it with --release)')
            [CompletionResult]::new('tethering', 'tethering', [CompletionResultType]::ParameterValue, 'Ask a native phone to open its tethering settings screen')
            [CompletionResult]::new('call', 'call', [CompletionResultType]::ParameterValue, 'Send a call control action to a native phone')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'handoverctl;native;peers' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'handoverctl;native;pending' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'handoverctl;native;pair' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'handoverctl;native;unpair' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'handoverctl;native;ping' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'handoverctl;native;ring' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'handoverctl;native;lock' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'handoverctl;native;keep-awake' {
            [CompletionResult]::new('--release', '--release', [CompletionResultType]::ParameterName, 'Release the phone wake lock instead of holding it')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'handoverctl;native;tethering' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'handoverctl;native;call' {
            [CompletionResult]::new('--confirm', '--confirm', [CompletionResultType]::ParameterName, 'Explicitly authorize placing a real phone call (required for place)')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'handoverctl;native;help' {
            [CompletionResult]::new('peers', 'peers', [CompletionResultType]::ParameterValue, 'List trusted native peers (id, name, certificate fingerprint)')
            [CompletionResult]::new('pending', 'pending', [CompletionResultType]::ParameterValue, 'List pending pairing requests (id, name, eight-digit code)')
            [CompletionResult]::new('pair', 'pair', [CompletionResultType]::ParameterValue, 'Approve a pending pairing request after comparing codes')
            [CompletionResult]::new('unpair', 'unpair', [CompletionResultType]::ParameterValue, 'Revoke a trusted native peer')
            [CompletionResult]::new('ping', 'ping', [CompletionResultType]::ParameterValue, 'Send a user-visible liveness ping to a native device name or ID')
            [CompletionResult]::new('ring', 'ring', [CompletionResultType]::ParameterValue, 'Ring and vibrate a native device selected by name or ID')
            [CompletionResult]::new('lock', 'lock', [CompletionResultType]::ParameterValue, 'Lock a native phone when device-admin access is enabled')
            [CompletionResult]::new('keep-awake', 'keep-awake', [CompletionResultType]::ParameterValue, 'Ask a native phone to hold its wake lock (or release it with --release)')
            [CompletionResult]::new('tethering', 'tethering', [CompletionResultType]::ParameterValue, 'Ask a native phone to open its tethering settings screen')
            [CompletionResult]::new('call', 'call', [CompletionResultType]::ParameterValue, 'Send a call control action to a native phone')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'handoverctl;native;help;peers' {
            break
        }
        'handoverctl;native;help;pending' {
            break
        }
        'handoverctl;native;help;pair' {
            break
        }
        'handoverctl;native;help;unpair' {
            break
        }
        'handoverctl;native;help;ping' {
            break
        }
        'handoverctl;native;help;ring' {
            break
        }
        'handoverctl;native;help;lock' {
            break
        }
        'handoverctl;native;help;keep-awake' {
            break
        }
        'handoverctl;native;help;tethering' {
            break
        }
        'handoverctl;native;help;call' {
            break
        }
        'handoverctl;native;help;help' {
            break
        }
        'handoverctl;notifications' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'handoverctl;contacts' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('list', 'list', [CompletionResultType]::ParameterValue, 'List the latest contacts snapshot held by handoverd')
            [CompletionResult]::new('sync', 'sync', [CompletionResultType]::ParameterValue, 'Ask one native phone to send a fresh contacts snapshot')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'handoverctl;contacts;list' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'handoverctl;contacts;sync' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'handoverctl;contacts;help' {
            [CompletionResult]::new('list', 'list', [CompletionResultType]::ParameterValue, 'List the latest contacts snapshot held by handoverd')
            [CompletionResult]::new('sync', 'sync', [CompletionResultType]::ParameterValue, 'Ask one native phone to send a fresh contacts snapshot')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'handoverctl;contacts;help;list' {
            break
        }
        'handoverctl;contacts;help;sync' {
            break
        }
        'handoverctl;contacts;help;help' {
            break
        }
        'handoverctl;media' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('play', 'play', [CompletionResultType]::ParameterValue, 'Start playback on one session')
            [CompletionResult]::new('pause', 'pause', [CompletionResultType]::ParameterValue, 'Pause playback on one session')
            [CompletionResult]::new('play-pause', 'play-pause', [CompletionResultType]::ParameterValue, 'Toggle playback on one session')
            [CompletionResult]::new('next', 'next', [CompletionResultType]::ParameterValue, 'Skip to the next item on one session')
            [CompletionResult]::new('previous', 'previous', [CompletionResultType]::ParameterValue, 'Return to the previous item on one session')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'handoverctl;media;play' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'handoverctl;media;pause' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'handoverctl;media;play-pause' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'handoverctl;media;next' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'handoverctl;media;previous' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'handoverctl;media;help' {
            [CompletionResult]::new('play', 'play', [CompletionResultType]::ParameterValue, 'Start playback on one session')
            [CompletionResult]::new('pause', 'pause', [CompletionResultType]::ParameterValue, 'Pause playback on one session')
            [CompletionResult]::new('play-pause', 'play-pause', [CompletionResultType]::ParameterValue, 'Toggle playback on one session')
            [CompletionResult]::new('next', 'next', [CompletionResultType]::ParameterValue, 'Skip to the next item on one session')
            [CompletionResult]::new('previous', 'previous', [CompletionResultType]::ParameterValue, 'Return to the previous item on one session')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'handoverctl;media;help;play' {
            break
        }
        'handoverctl;media;help;pause' {
            break
        }
        'handoverctl;media;help;play-pause' {
            break
        }
        'handoverctl;media;help;next' {
            break
        }
        'handoverctl;media;help;previous' {
            break
        }
        'handoverctl;media;help;help' {
            break
        }
        'handoverctl;monitor' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'handoverctl;send-url' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'handoverctl;notify' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'handoverctl;clipboard' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'handoverctl;send-file' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'handoverctl;screensaver' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'handoverctl;clipboard-mirror' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'handoverctl;clipboard-history' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('list', 'list', [CompletionResultType]::ParameterValue, 'List recent and pinned entries (text previews only)')
            [CompletionResult]::new('save', 'save', [CompletionResultType]::ParameterValue, 'Save a new pinned string')
            [CompletionResult]::new('pin', 'pin', [CompletionResultType]::ParameterValue, 'Pin an existing entry by id')
            [CompletionResult]::new('unpin', 'unpin', [CompletionResultType]::ParameterValue, 'Unpin an existing entry by id')
            [CompletionResult]::new('copy', 'copy', [CompletionResultType]::ParameterValue, 'Copy an entry into the Linux clipboard')
            [CompletionResult]::new('clear', 'clear', [CompletionResultType]::ParameterValue, 'Remove recent entries, retaining pins unless --all is passed')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'handoverctl;clipboard-history;list' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'handoverctl;clipboard-history;save' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'handoverctl;clipboard-history;pin' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'handoverctl;clipboard-history;unpin' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'handoverctl;clipboard-history;copy' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'handoverctl;clipboard-history;clear' {
            [CompletionResult]::new('--all', '--all', [CompletionResultType]::ParameterName, 'Also remove pinned entries')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'handoverctl;clipboard-history;help' {
            [CompletionResult]::new('list', 'list', [CompletionResultType]::ParameterValue, 'List recent and pinned entries (text previews only)')
            [CompletionResult]::new('save', 'save', [CompletionResultType]::ParameterValue, 'Save a new pinned string')
            [CompletionResult]::new('pin', 'pin', [CompletionResultType]::ParameterValue, 'Pin an existing entry by id')
            [CompletionResult]::new('unpin', 'unpin', [CompletionResultType]::ParameterValue, 'Unpin an existing entry by id')
            [CompletionResult]::new('copy', 'copy', [CompletionResultType]::ParameterValue, 'Copy an entry into the Linux clipboard')
            [CompletionResult]::new('clear', 'clear', [CompletionResultType]::ParameterValue, 'Remove recent entries, retaining pins unless --all is passed')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'handoverctl;clipboard-history;help;list' {
            break
        }
        'handoverctl;clipboard-history;help;save' {
            break
        }
        'handoverctl;clipboard-history;help;pin' {
            break
        }
        'handoverctl;clipboard-history;help;unpin' {
            break
        }
        'handoverctl;clipboard-history;help;copy' {
            break
        }
        'handoverctl;clipboard-history;help;clear' {
            break
        }
        'handoverctl;clipboard-history;help;help' {
            break
        }
        'handoverctl;cancel-share' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'handoverctl;custom' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('list', 'list', [CompletionResultType]::ParameterValue, 'List allowlisted desktop commands with their fixed argv')
            [CompletionResult]::new('run', 'run', [CompletionResultType]::ParameterValue, 'Run one allowlisted desktop command by name')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'handoverctl;custom;list' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'handoverctl;custom;run' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'handoverctl;custom;help' {
            [CompletionResult]::new('list', 'list', [CompletionResultType]::ParameterValue, 'List allowlisted desktop commands with their fixed argv')
            [CompletionResult]::new('run', 'run', [CompletionResultType]::ParameterValue, 'Run one allowlisted desktop command by name')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'handoverctl;custom;help;list' {
            break
        }
        'handoverctl;custom;help;run' {
            break
        }
        'handoverctl;custom;help;help' {
            break
        }
        'handoverctl;calls' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'handoverctl;messages' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('accounts', 'accounts', [CompletionResultType]::ParameterValue, 'List messaging accounts with connection state')
            [CompletionResult]::new('conversations', 'conversations', [CompletionResultType]::ParameterValue, 'List conversations for one account')
            [CompletionResult]::new('history', 'history', [CompletionResultType]::ParameterValue, 'Show message history for one conversation (ACCOUNT:THREAD or THREAD)')
            [CompletionResult]::new('send', 'send', [CompletionResultType]::ParameterValue, 'Send a text message (accepted, not delivered)')
            [CompletionResult]::new('send-file', 'send-file', [CompletionResultType]::ParameterValue, 'Send a file attachment with an optional caption')
            [CompletionResult]::new('reply', 'reply', [CompletionResultType]::ParameterValue, 'Reply to a message (ACCOUNT:THREAD:MESSAGE or THREAD:MESSAGE ...)')
            [CompletionResult]::new('react', 'react', [CompletionResultType]::ParameterValue, 'Add a reaction to a message')
            [CompletionResult]::new('unreact', 'unreact', [CompletionResultType]::ParameterValue, 'Remove a reaction from a message')
            [CompletionResult]::new('read', 'read', [CompletionResultType]::ParameterValue, 'Mark a conversation read (optionally up to one message)')
            [CompletionResult]::new('typing', 'typing', [CompletionResultType]::ParameterValue, 'Send a typing-start ping (no typing-stop exists upstream)')
            [CompletionResult]::new('delete', 'delete', [CompletionResultType]::ParameterValue, 'Delete one own message')
            [CompletionResult]::new('open', 'open', [CompletionResultType]::ParameterValue, 'Open or create a conversation with addresses (phone numbers/emails)')
            [CompletionResult]::new('login', 'login', [CompletionResultType]::ParameterValue, 'Log in: read a credential bundle from a file or stdin, never argv')
            [CompletionResult]::new('logout', 'logout', [CompletionResultType]::ParameterValue, 'Log out and revoke helper access')
            [CompletionResult]::new('sync', 'sync', [CompletionResultType]::ParameterValue, 'Ask the helper to re-emit authoritative state for one account')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'handoverctl;messages;accounts' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'handoverctl;messages;conversations' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'handoverctl;messages;history' {
            [CompletionResult]::new('--limit', '--limit', [CompletionResultType]::ParameterName, 'Maximum messages to show')
            [CompletionResult]::new('--cursor', '--cursor', [CompletionResultType]::ParameterName, 'Cursor printed by a previous history page')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'handoverctl;messages;send' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'handoverctl;messages;send-file' {
            [CompletionResult]::new('--caption', '--caption', [CompletionResultType]::ParameterName, 'Optional caption sent with the attachment')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'handoverctl;messages;reply' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'handoverctl;messages;react' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'handoverctl;messages;unreact' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'handoverctl;messages;read' {
            [CompletionResult]::new('--message', '--message', [CompletionResultType]::ParameterName, 'Message selector marking the read point')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'handoverctl;messages;typing' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'handoverctl;messages;delete' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'handoverctl;messages;open' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'handoverctl;messages;login' {
            [CompletionResult]::new('--from-file', '--from-file', [CompletionResultType]::ParameterName, 'Read the credential bundle from PATH instead of stdin')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'handoverctl;messages;logout' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'handoverctl;messages;sync' {
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'handoverctl;messages;help' {
            [CompletionResult]::new('accounts', 'accounts', [CompletionResultType]::ParameterValue, 'List messaging accounts with connection state')
            [CompletionResult]::new('conversations', 'conversations', [CompletionResultType]::ParameterValue, 'List conversations for one account')
            [CompletionResult]::new('history', 'history', [CompletionResultType]::ParameterValue, 'Show message history for one conversation (ACCOUNT:THREAD or THREAD)')
            [CompletionResult]::new('send', 'send', [CompletionResultType]::ParameterValue, 'Send a text message (accepted, not delivered)')
            [CompletionResult]::new('send-file', 'send-file', [CompletionResultType]::ParameterValue, 'Send a file attachment with an optional caption')
            [CompletionResult]::new('reply', 'reply', [CompletionResultType]::ParameterValue, 'Reply to a message (ACCOUNT:THREAD:MESSAGE or THREAD:MESSAGE ...)')
            [CompletionResult]::new('react', 'react', [CompletionResultType]::ParameterValue, 'Add a reaction to a message')
            [CompletionResult]::new('unreact', 'unreact', [CompletionResultType]::ParameterValue, 'Remove a reaction from a message')
            [CompletionResult]::new('read', 'read', [CompletionResultType]::ParameterValue, 'Mark a conversation read (optionally up to one message)')
            [CompletionResult]::new('typing', 'typing', [CompletionResultType]::ParameterValue, 'Send a typing-start ping (no typing-stop exists upstream)')
            [CompletionResult]::new('delete', 'delete', [CompletionResultType]::ParameterValue, 'Delete one own message')
            [CompletionResult]::new('open', 'open', [CompletionResultType]::ParameterValue, 'Open or create a conversation with addresses (phone numbers/emails)')
            [CompletionResult]::new('login', 'login', [CompletionResultType]::ParameterValue, 'Log in: read a credential bundle from a file or stdin, never argv')
            [CompletionResult]::new('logout', 'logout', [CompletionResultType]::ParameterValue, 'Log out and revoke helper access')
            [CompletionResult]::new('sync', 'sync', [CompletionResultType]::ParameterValue, 'Ask the helper to re-emit authoritative state for one account')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'handoverctl;messages;help;accounts' {
            break
        }
        'handoverctl;messages;help;conversations' {
            break
        }
        'handoverctl;messages;help;history' {
            break
        }
        'handoverctl;messages;help;send' {
            break
        }
        'handoverctl;messages;help;send-file' {
            break
        }
        'handoverctl;messages;help;reply' {
            break
        }
        'handoverctl;messages;help;react' {
            break
        }
        'handoverctl;messages;help;unreact' {
            break
        }
        'handoverctl;messages;help;read' {
            break
        }
        'handoverctl;messages;help;typing' {
            break
        }
        'handoverctl;messages;help;delete' {
            break
        }
        'handoverctl;messages;help;open' {
            break
        }
        'handoverctl;messages;help;login' {
            break
        }
        'handoverctl;messages;help;logout' {
            break
        }
        'handoverctl;messages;help;sync' {
            break
        }
        'handoverctl;messages;help;help' {
            break
        }
        'handoverctl;help' {
            [CompletionResult]::new('devices', 'devices', [CompletionResultType]::ParameterValue, 'List devices known to handoverd')
            [CompletionResult]::new('native', 'native', [CompletionResultType]::ParameterValue, 'Inspect and manage native Android pairing and device commands')
            [CompletionResult]::new('notifications', 'notifications', [CompletionResultType]::ParameterValue, 'List active remote notifications from the daemon snapshot')
            [CompletionResult]::new('contacts', 'contacts', [CompletionResultType]::ParameterValue, 'List or request an on-demand native contacts snapshot')
            [CompletionResult]::new('media', 'media', [CompletionResultType]::ParameterValue, 'List media sessions, or send one playback command')
            [CompletionResult]::new('monitor', 'monitor', [CompletionResultType]::ParameterValue, 'Print live normalized device and notification changes')
            [CompletionResult]::new('send-url', 'send-url', [CompletionResultType]::ParameterValue, 'Send a URL to one paired device')
            [CompletionResult]::new('notify', 'notify', [CompletionResultType]::ParameterValue, 'Send a notification to one native phone')
            [CompletionResult]::new('clipboard', 'clipboard', [CompletionResultType]::ParameterValue, 'Set the clipboard on one native phone')
            [CompletionResult]::new('send-file', 'send-file', [CompletionResultType]::ParameterValue, 'Send one local file to a paired device')
            [CompletionResult]::new('screensaver', 'screensaver', [CompletionResultType]::ParameterValue, 'Control the desktop idle inhibitor: inhibit, release, or follow')
            [CompletionResult]::new('clipboard-mirror', 'clipboard-mirror', [CompletionResultType]::ParameterValue, 'Get or set Linux-to-phone background clipboard mirroring (off by default)')
            [CompletionResult]::new('clipboard-history', 'clipboard-history', [CompletionResultType]::ParameterValue, 'List, pin, copy, and clear phone-to-Linux clipboard history')
            [CompletionResult]::new('cancel-share', 'cancel-share', [CompletionResultType]::ParameterValue, 'Cancel a queued native share that has not started streaming')
            [CompletionResult]::new('custom', 'custom', [CompletionResultType]::ParameterValue, 'List or run allowlisted desktop commands')
            [CompletionResult]::new('calls', 'calls', [CompletionResultType]::ParameterValue, 'Print current normalized call state for one device')
            [CompletionResult]::new('messages', 'messages', [CompletionResultType]::ParameterValue, 'Inspect and use messaging accounts and conversations')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'handoverctl;help;devices' {
            break
        }
        'handoverctl;help;native' {
            [CompletionResult]::new('peers', 'peers', [CompletionResultType]::ParameterValue, 'List trusted native peers (id, name, certificate fingerprint)')
            [CompletionResult]::new('pending', 'pending', [CompletionResultType]::ParameterValue, 'List pending pairing requests (id, name, eight-digit code)')
            [CompletionResult]::new('pair', 'pair', [CompletionResultType]::ParameterValue, 'Approve a pending pairing request after comparing codes')
            [CompletionResult]::new('unpair', 'unpair', [CompletionResultType]::ParameterValue, 'Revoke a trusted native peer')
            [CompletionResult]::new('ping', 'ping', [CompletionResultType]::ParameterValue, 'Send a user-visible liveness ping to a native device name or ID')
            [CompletionResult]::new('ring', 'ring', [CompletionResultType]::ParameterValue, 'Ring and vibrate a native device selected by name or ID')
            [CompletionResult]::new('lock', 'lock', [CompletionResultType]::ParameterValue, 'Lock a native phone when device-admin access is enabled')
            [CompletionResult]::new('keep-awake', 'keep-awake', [CompletionResultType]::ParameterValue, 'Ask a native phone to hold its wake lock (or release it with --release)')
            [CompletionResult]::new('tethering', 'tethering', [CompletionResultType]::ParameterValue, 'Ask a native phone to open its tethering settings screen')
            [CompletionResult]::new('call', 'call', [CompletionResultType]::ParameterValue, 'Send a call control action to a native phone')
            break
        }
        'handoverctl;help;native;peers' {
            break
        }
        'handoverctl;help;native;pending' {
            break
        }
        'handoverctl;help;native;pair' {
            break
        }
        'handoverctl;help;native;unpair' {
            break
        }
        'handoverctl;help;native;ping' {
            break
        }
        'handoverctl;help;native;ring' {
            break
        }
        'handoverctl;help;native;lock' {
            break
        }
        'handoverctl;help;native;keep-awake' {
            break
        }
        'handoverctl;help;native;tethering' {
            break
        }
        'handoverctl;help;native;call' {
            break
        }
        'handoverctl;help;notifications' {
            break
        }
        'handoverctl;help;contacts' {
            [CompletionResult]::new('list', 'list', [CompletionResultType]::ParameterValue, 'List the latest contacts snapshot held by handoverd')
            [CompletionResult]::new('sync', 'sync', [CompletionResultType]::ParameterValue, 'Ask one native phone to send a fresh contacts snapshot')
            break
        }
        'handoverctl;help;contacts;list' {
            break
        }
        'handoverctl;help;contacts;sync' {
            break
        }
        'handoverctl;help;media' {
            [CompletionResult]::new('play', 'play', [CompletionResultType]::ParameterValue, 'Start playback on one session')
            [CompletionResult]::new('pause', 'pause', [CompletionResultType]::ParameterValue, 'Pause playback on one session')
            [CompletionResult]::new('play-pause', 'play-pause', [CompletionResultType]::ParameterValue, 'Toggle playback on one session')
            [CompletionResult]::new('next', 'next', [CompletionResultType]::ParameterValue, 'Skip to the next item on one session')
            [CompletionResult]::new('previous', 'previous', [CompletionResultType]::ParameterValue, 'Return to the previous item on one session')
            break
        }
        'handoverctl;help;media;play' {
            break
        }
        'handoverctl;help;media;pause' {
            break
        }
        'handoverctl;help;media;play-pause' {
            break
        }
        'handoverctl;help;media;next' {
            break
        }
        'handoverctl;help;media;previous' {
            break
        }
        'handoverctl;help;monitor' {
            break
        }
        'handoverctl;help;send-url' {
            break
        }
        'handoverctl;help;notify' {
            break
        }
        'handoverctl;help;clipboard' {
            break
        }
        'handoverctl;help;send-file' {
            break
        }
        'handoverctl;help;screensaver' {
            break
        }
        'handoverctl;help;clipboard-mirror' {
            break
        }
        'handoverctl;help;clipboard-history' {
            [CompletionResult]::new('list', 'list', [CompletionResultType]::ParameterValue, 'List recent and pinned entries (text previews only)')
            [CompletionResult]::new('save', 'save', [CompletionResultType]::ParameterValue, 'Save a new pinned string')
            [CompletionResult]::new('pin', 'pin', [CompletionResultType]::ParameterValue, 'Pin an existing entry by id')
            [CompletionResult]::new('unpin', 'unpin', [CompletionResultType]::ParameterValue, 'Unpin an existing entry by id')
            [CompletionResult]::new('copy', 'copy', [CompletionResultType]::ParameterValue, 'Copy an entry into the Linux clipboard')
            [CompletionResult]::new('clear', 'clear', [CompletionResultType]::ParameterValue, 'Remove recent entries, retaining pins unless --all is passed')
            break
        }
        'handoverctl;help;clipboard-history;list' {
            break
        }
        'handoverctl;help;clipboard-history;save' {
            break
        }
        'handoverctl;help;clipboard-history;pin' {
            break
        }
        'handoverctl;help;clipboard-history;unpin' {
            break
        }
        'handoverctl;help;clipboard-history;copy' {
            break
        }
        'handoverctl;help;clipboard-history;clear' {
            break
        }
        'handoverctl;help;cancel-share' {
            break
        }
        'handoverctl;help;custom' {
            [CompletionResult]::new('list', 'list', [CompletionResultType]::ParameterValue, 'List allowlisted desktop commands with their fixed argv')
            [CompletionResult]::new('run', 'run', [CompletionResultType]::ParameterValue, 'Run one allowlisted desktop command by name')
            break
        }
        'handoverctl;help;custom;list' {
            break
        }
        'handoverctl;help;custom;run' {
            break
        }
        'handoverctl;help;calls' {
            break
        }
        'handoverctl;help;messages' {
            [CompletionResult]::new('accounts', 'accounts', [CompletionResultType]::ParameterValue, 'List messaging accounts with connection state')
            [CompletionResult]::new('conversations', 'conversations', [CompletionResultType]::ParameterValue, 'List conversations for one account')
            [CompletionResult]::new('history', 'history', [CompletionResultType]::ParameterValue, 'Show message history for one conversation (ACCOUNT:THREAD or THREAD)')
            [CompletionResult]::new('send', 'send', [CompletionResultType]::ParameterValue, 'Send a text message (accepted, not delivered)')
            [CompletionResult]::new('send-file', 'send-file', [CompletionResultType]::ParameterValue, 'Send a file attachment with an optional caption')
            [CompletionResult]::new('reply', 'reply', [CompletionResultType]::ParameterValue, 'Reply to a message (ACCOUNT:THREAD:MESSAGE or THREAD:MESSAGE ...)')
            [CompletionResult]::new('react', 'react', [CompletionResultType]::ParameterValue, 'Add a reaction to a message')
            [CompletionResult]::new('unreact', 'unreact', [CompletionResultType]::ParameterValue, 'Remove a reaction from a message')
            [CompletionResult]::new('read', 'read', [CompletionResultType]::ParameterValue, 'Mark a conversation read (optionally up to one message)')
            [CompletionResult]::new('typing', 'typing', [CompletionResultType]::ParameterValue, 'Send a typing-start ping (no typing-stop exists upstream)')
            [CompletionResult]::new('delete', 'delete', [CompletionResultType]::ParameterValue, 'Delete one own message')
            [CompletionResult]::new('open', 'open', [CompletionResultType]::ParameterValue, 'Open or create a conversation with addresses (phone numbers/emails)')
            [CompletionResult]::new('login', 'login', [CompletionResultType]::ParameterValue, 'Log in: read a credential bundle from a file or stdin, never argv')
            [CompletionResult]::new('logout', 'logout', [CompletionResultType]::ParameterValue, 'Log out and revoke helper access')
            [CompletionResult]::new('sync', 'sync', [CompletionResultType]::ParameterValue, 'Ask the helper to re-emit authoritative state for one account')
            break
        }
        'handoverctl;help;messages;accounts' {
            break
        }
        'handoverctl;help;messages;conversations' {
            break
        }
        'handoverctl;help;messages;history' {
            break
        }
        'handoverctl;help;messages;send' {
            break
        }
        'handoverctl;help;messages;send-file' {
            break
        }
        'handoverctl;help;messages;reply' {
            break
        }
        'handoverctl;help;messages;react' {
            break
        }
        'handoverctl;help;messages;unreact' {
            break
        }
        'handoverctl;help;messages;read' {
            break
        }
        'handoverctl;help;messages;typing' {
            break
        }
        'handoverctl;help;messages;delete' {
            break
        }
        'handoverctl;help;messages;open' {
            break
        }
        'handoverctl;help;messages;login' {
            break
        }
        'handoverctl;help;messages;logout' {
            break
        }
        'handoverctl;help;messages;sync' {
            break
        }
        'handoverctl;help;help' {
            break
        }
    })

    $completions.Where{ $_.CompletionText -like "$wordToComplete*" } |
        Sort-Object -Property ListItemText
}
