# Handover roadmap

Four milestones. Each one should be usable on its own. Later work does not
block an earlier milestone unless skipping it would force a rewrite.

## Milestone 1: replace KDE Connect (done)

A user can disable KDE Connect and keep the Android and Linux workflows
Handover intends to support. Native Handover is the normal connection path.
KDE Connect remains an optional compatibility backend.

Caveats from the device pass are in `docs/KNOWN_LIMITATIONS.md`. Do not reopen
this milestone for first-party Messages apps, independent Google protocol work,
continuity features, or 1.0 APIs.

## Milestone 2: make connectivity automatic

The user should not manage the connection after pairing.

Work: reconnect latency, suspend/resume, Android process and service recovery,
network transitions, stale-session rejection, capability renegotiation, stable
identity, state restoration, idle CPU and memory, Android battery, installation
and upgrades, desktop integration.

Done when common network and process failures recover without user action, one
physical phone stays one identity, multi-day sessions stay usable, and install
does not require maintainer folklore.

## Milestone 3: own communications

Linux-native model for calls, contacts, SMS, and RCS. Clients use Handover
types, not provider objects.

`handover-gmessages` stays the Google protocol engine for now. Handover owns
supervision, normalization, persistence policy, lifecycle, the helper contract,
and the Linux-facing model. Replace libgm later behind that provider boundary
without changing clients.

A first-party Messages app, if built, talks only to public Handover messaging
IPC. It does not embed Google protocol code.

Done when daily SMS and RCS from Linux is practical, Google types do not leak
into clients, and the Google provider can change without rewriting `handoverd`.

## Milestone 4: Android ecosystem integration

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
It does not wait for Milestone 4.

```text
0.3.x   native path, Milestone 1 closed
0.4.x   Milestone 2: reconnect, packaging, daily-driver reliability
0.5+    communications, first-party apps, broader desktop integration
1.0     stable public device-integration API
```

Version for compatibility, not a calendar.

## Current priority

Quiet public inspection of this MIT repo. Cold-clone install. Then Milestone 2:
reconnect, packaging, and whatever daily use breaks first.
