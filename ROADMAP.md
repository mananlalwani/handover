# Handover roadmap

## Milestone 1: native device connection, complete

Native Handover is the primary Android/Linux connection. KDE Connect remains an
optional compatibility backend. Device pairing, notifications, clipboard, file
sharing, and media controls use backend-independent Handover models.

## Milestone 2: own communications, complete

Calls, contacts, SMS, and RCS use normalized Handover models. The independent MIT
Google Messages client is the default messaging provider, with pairing, live
updates, paged history, text and media sends, delivery evidence, and session
recovery. It requires no mautrix dependency or separate repository.

Product limits remain in [known limitations](docs/KNOWN_LIMITATIONS.md).

## Milestone 3: Android ecosystem integration

The next stage brings Android capabilities into Linux launchers, file managers,
browsers, notifications, media controls, and shell panels. Activity handoff and
draft continuation build on these shared services.

First-party Messages, Calls, or Contacts apps are useful where a shell panel is
insufficient. They remain replaceable clients of the same public IPC.

## Versioning

Handover remains on `0.x` while public interfaces may change. Version `1.0` means
users and integrators can depend on the documented public behavior. Milestone
completion does not itself determine release numbers.
