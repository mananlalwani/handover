# Handover roadmap

Three milestones. Each one is usable on its own. Later work does not block an
earlier milestone unless skipping it would force a rewrite.

## Milestone 1: replace KDE Connect (done)

A user can disable KDE Connect and keep the Android and Linux workflows
Handover intends to support. Native Handover is the normal connection path.
KDE Connect remains an optional compatibility backend.

Caveats from the device pass are in `docs/KNOWN_LIMITATIONS.md`. Do not reopen
this milestone for first-party Messages apps, independent Google protocol work,
continuity features, or 1.0 APIs.

## Milestone 2: own communications

Linux-native model for calls, contacts, SMS, and RCS. Clients use Handover
types, not provider objects.

Implement Handover's own Google Messages protocol client in `google-messages/`
without
`mautrix-gmessages` or copied mautrix implementation code. Keep Google wire
formats, authentication, pairing, encryption, and relay behavior below the
normalized helper contract. The existing adapter remains a temporary bootstrap
backend while the independent client is developed and verified.

Start by proving fresh pairing and read-only conversation/history retrieval.
Then implement live updates, text and media sends, provider-backed outcomes, and
session recovery before switching the normal communications path.

A first-party Messages app, if built, talks only to public Handover messaging
IPC. It does not embed Google protocol code.

Done when daily SMS and RCS from Linux works through the independently authored
client without a mautrix dependency, Google types stay out of public clients,
and pairing, delivery evidence, media, expiry, and recovery are live-tested.
Normalizing or hardening the existing adapter alone does not close this milestone.

## Milestone 3: Android ecosystem integration

Expose Android capabilities in existing Linux workflows: launcher, file
manager, browser, notifications, media, shell panels. Continuity (activity
handoff, draft continuation) comes after the shared services are solid.

First-party Messages, Calls, or Contacts apps exist only if a shell panel is
not enough. They remain replaceable clients of the same public IPC.

Done when people can handle common communications and content for days without
manually deciding which device owns the activity. Matching every Apple
Continuity feature is not required.

## Development rules

1. Do not delay reliability for continuity features.
2. Keep provider protocols behind stable contracts.
3. First-party apps are not provider implementations.
4. First-party and third-party clients share the public model.
5. Extract shared helpers after the same rules appear twice, not before.
6. A feature is not done until failure cases are tested.
7. Public APIs and on-disk formats change slower than internals.
8. Turn reproducible bugs into regression tests when practical.

## Versioning

The project is `0.3.x`. Stay on `0.x` while public interfaces may change.
`1.0.0` is when users and integrators can depend on the promised behavior.
It does not wait for Milestone 3.

```text
0.3.x   native path, Milestone 1 closed
0.4.x   Milestone 2: communications
0.5+    Milestone 3: Android ecosystem integration
1.0     stable public device-integration API
```

Version for compatibility, not a calendar.

## Current priority

Milestone 2 prioritizes replacing the mautrix-based Google protocol engine with
Handover's own implementation behind the existing helper contract. Calls,
contacts, and public messaging clients remain interoperable during the transition.
