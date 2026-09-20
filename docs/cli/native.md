# Native pairing and phone commands

Commands under `handoverctl native` manage the native Android connection.
Peer selection accepts a peer ID or, when unique, a device name. Pairing
codes are compared by the operator before approval.

```
handoverctl native peers
handoverctl native pending
handoverctl native pair ID CODE
handoverctl native unpair ID
handoverctl native ping DEVICE
handoverctl native ring DEVICE
handoverctl native lock DEVICE
handoverctl native keep-awake DEVICE [--release]
handoverctl native tethering DEVICE
handoverctl native call DEVICE ACTION [ADDRESS] [--confirm]
```

## peers

Lists trusted native peers: id, name, and certificate fingerprint.
No arguments.

## pending

Lists pending pairing requests: id, name, and eight-digit code. Compare
the code shown here with the code on the phone before approving. No
arguments.

## pair

```
handoverctl native pair <pending-id> <eight-digit-code>
```

Approves a pending pairing request. Records the approval on Linux. The
phone still confirms the pairing, so approval alone does not mean the
devices are paired. Both ID and CODE are required.

## unpair

```
handoverctl native unpair <peer-id>
```

Revokes a trusted native peer. Remove the connection from the phone as
well when the peer should not reconnect.

## ping

Sends a user-visible liveness ping to the phone. The request is queued.
Use `handoverctl monitor` to observe the Android result.

## ring

Rings and vibrates the phone. The request is queued. Use `monitor` to
observe the Android result.

## lock

Locks the phone. Needs device-admin access on the phone. The request is
queued. Use `monitor` to observe the Android result.

## keep-awake

Asks the phone to hold its wake lock. With `--release`, releases the
wake lock instead. The request is queued. Use `monitor` to observe the
Android result.

## tethering

Asks the phone to open its tethering settings screen. The request is
queued. Use `monitor` to observe the Android result.

## call

```
handoverctl native call DEVICE ACTION [ADDRESS] [--confirm]
```

Sends a call control action to the phone. The command is accepted, not
confirmed. The `place` action dials a real number and requires
`--confirm`. Without `--confirm`, `place` exits 1 and dials nothing.

## calls

```
handoverctl calls DEVICE
```

Not under `native`. Prints current normalized call state for one device
from the daemon snapshot. See `docs/cli/handoverctl.md`.
