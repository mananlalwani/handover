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

Command acceptance currently means **queued for the live session**. It is not an
Android acknowledgement or a confirmed effect. Android/OEM restrictions can
reject an operation. Answer/decline/hangup effects still need user-authorized
live validation; no automatic calls should be used for testing. Multi-call,
per-SIM selection, hold, DTMF, per-call identities, VoIP and automated audio
routing are not implemented by this aggregate telephony interface. The current
observer follows Android's default telephony subscription; dual-SIM aggregate
state is not guaranteed. Phone-side rejection reasons are not yet correlated
back to individual desktop commands.

Controls are bound to the phone's reported state generation. The native session
drops a queued command after any newer phone state report, and Android rechecks
the same generation before execution. This prevents a delayed answer, decline,
or hangup from acting on a later call.
