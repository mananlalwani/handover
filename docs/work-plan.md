# Remaining work

## Phone and desktop validation

Run the [phone checklist](phone-validation.md) and
[Android security checks](../android/SECURITY_DEVICE_VALIDATION.md) on matching
builds. Priorities are interrupted downloads and resume, notification actions,
network changes and suspend, Google Messages recovery, and audible call audio.
Record the tested build and whether each result came from a phone, emulator, or
fixture. Keep private logs and device identifiers out of the repository.

## Google Messages

- Resolve the incoming RCS registration failure using a successful first-party
  registration for comparison. See [typing](gmessages-typing.md).
- Establish first-party request, completion, and reconciliation evidence before
  adding conversation creation, message deletion, or outgoing typing. See the
  [protocol evidence](../google-messages/protocol-evidence.md).
- Verify multi-attachment acceptance and recipient delivery on a real account.

## Compatibility and distribution

- Keep [IPC protocol 1](ipc.md) experimental until the compatibility policy can
  be supported for external clients.
- Validate Bluetooth call audibility, routing, and recovery with a physical
  phone before removing its experimental status.
- Publish Arch packages to the AUR after maintaining reproducible install and
  upgrade checks. Additional distribution formats are optional follow-up work.

Implemented behavior and current limits belong in the [user guide](user-guide.md)
and [known limitations](KNOWN_LIMITATIONS.md). Test commands belong in
[Contributing](../CONTRIBUTING.md).
