# Native cellular calls

Android observes aggregate telephony state: `unknown`, `idle`, `ringing`, and
`off_hook`. Off-hook includes dialing; it is **not** confirmation that somebody
answered. No call log, caller number or contact permission is collected.

The daemon owns the current per-device call snapshot (`calls.list`, subscription
`calls`, `call_updated`, `call_removed`). Disconnect removes the state; absence
is not idle. Commands use backend-independent `calls.control` with `device_id`,
`action` and optional `address`. The legacy `native.call` method remains available.
Android rechecks phone permissions and call phase when executing each command.

## Use

- Android: enable the Handover connection and grant call permissions. The
  capability screen shows call-state, placement and answer/end permissions
  individually. Already-granted access produces a visible message.
- Linux: `handoverctl calls SM-S948U1` reports state and available actions.
- Calls page: Call requires confirmation; Answer/Decline/Hang up appear enabled
  only when advertised by the connected phone.
- CLI: `handoverctl native call PEER_ID place NUMBER --confirm` places a **real
  call**. Answer, decline and hangup use the same command without a number.

Do not test placement using a supposedly fictitious or emergency number on a
live phone. Use isolated tests with transport disabled. Emergency recognition is
Android's local telephony check immediately before placement, not a hardcoded
list. Service codes, URI syntax, pauses and extensions are excluded.

## Audio and limitations

Audio stays on established Bluetooth HFP / PipeWire, not the native data socket.
`calls.audio` performs a bounded read-only local audio inspection. The page
refreshes it on entry, call-state changes, or explicit Refresh. It never changes
Bluetooth pairing, audio profiles, defaults, microphone routing or links.

Audio observations are **host-wide**, not associated with the selected native
phone. A selected audio-gateway profile does not prove an audio stream; running
input/output endpoints do not prove audibility or speaker/microphone routing.
Unavailable inspection is shown as unavailable, not disconnected.

Command queuing currently returns a `request_id` without promising execution.
Android then reports a correlated verdict (`call_result` / `call_command_result`)
for accepted execution, permission denial, stale state, emergency-number
rejection, invalid address, wrong phase, or platform rejection. `accepted`
means Android's telephony API accepted the operation, never that a call
connected or the remote party answered. Answer/decline/hangup effects still
need deliberate user-authorized validation; no automatic calls should be used
for testing. Ringing takes precedence over off-hook, off-hook takes precedence
over idle, and the observer re-registers when Android reports subscription
changes; no subscription identifiers, phone numbers, or call logs are exposed.
Multi-call, hold, DTMF, per-call selection, per-call identities, VoIP, and
automated audio routing are not implemented by this aggregate telephony
interface.

Controls are bound to the phone's reported state generation. The native session
drops a queued command after any newer phone state report, and Android rechecks
the same generation before execution. This prevents a delayed answer, decline,
or hangup from acting on a later call.
