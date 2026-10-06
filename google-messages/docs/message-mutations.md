# Native replies, reactions, and unpairing

Implemented 2026-10-05 with automated wire and local mock-phone coverage. The
user subsequently confirmed replies and reactions on the phone, and confirmed
that Disconnect removed Handover from the phone's linked devices. These live
results apply to the tested account and desktop.

## First-party evidence

The anonymously fetched public client module has SHA-256
`4120a43a939d8148df97560a1eabc74d00226c2cc21006effaaa5be52a812b05` and size 5007805 bytes.
Raw JavaScript remains outside Git. Evidence comes from the
[Google client module](https://www.gstatic.com/_/messagesweb/_/js/k=messagesweb.mw.en.mz3Ni9GrZJM.O/am=iAAAQwACAEA/d=1/rs=AIp04d-7vAXPgfQIEPL92BWrMBtX0qFdZQ/m=PQKY2b,KUe0Gd,sy6,mws_lottie,mw_one_google_bar,e2QYhf,sy8,sym,syv,sy1m,sy1x,sy1w,sy1v,syn,syj,syo,syp,sya,syy,syb,syf,sye,syh,syi,syk,syl,syg,sy10,sys,syu,sy11,syx,sy12,sy13,sy14,syz,sy15,sy16,sy1n,sy1f,sy1r,sy1s,sy1t,syc,syd,sy1j,sy1a,sy9,sy18,syr,sy1p,syq,syt,syw,sy17,sy19,sy1b,sy1o,sy1y,sy1c,sy1d,sy1e,sy1h,sy1g,sy1i,sy1k,sy1l,sy1q,sy1u,sy1z,sy20,sy21,sy22,sy23,sy24,mw_conversation,sy25,mw_lazy_loaded_service_module,sy26,sy2a,sy28,sy2d,sy2e,sy27,sy29,sy2g,sy2b,sy2c,sy2f,mw_bugle,mw_dark_theme,sy2l,sy2i,sy2j,sy2k,sy2h,sy2m,sy2n,sy2q,sy2r,sy2o,sy2p,sy2s,sy2t,mw_EBB01A26F64706F3,sy2u,mw_emoji_metadata_loader_current_module,mw_emoji_metadata_loader_new_module,mw_hammerjs,mw_high_contrast_theme,mw_pnl,sy2v,mw_rcs_chat,sy35,sy2w,sy2x,sy2y,sy2z,sy30,sy31,sy32,sy33,sy34,mw_sat_fg,mw_satellite_account_switcher,mw_satelliteLoader,sy36,sy37,sy38,sy39,mw_web_only,sy3a,Lgtfhb,OhuMNb,pMnTZd,ze08tf,mlEf5e,sy3b,sy3c,sy3d,ogYuJ,sy3e,sy3f,sy3g,sy3h,FoZ3qf,sy3j,sy3i,S2QjCb,sy3k,xnxcec,sy3l,sy3m,sy3n,EEaDoe,OGXG3b,sy3o,MqwFTd,sy3p,WMDsbd,kLTRQd,FjL6pd,xMgyvf,rBgCWd,sy3q,KYjYBe,sy3r,sy3s,sy3t,NkL5xe,fZRov). No adapter or AGPL code was used.
Offsets refer to this capture.

| Operation | Observed fields and dispatch | Source offset |
| --- | --- | --- |
| Reply | SendMessage field 8, nested field 1 is the target message ID | 1044884 in the earlier independent bootstrap; same `Wo` path in the full module |
| Reaction | Action 38; target field 1, emoji object field 2, operation field 3; response enum field 1 | 1049737 |
| Emoji | Nested string field 1 and mapped enum field 2; unknown strings use enum 8 | 995999, 1049737 |
| Gaia unpair | Action 46; pairing attempt ID field 1; response boolean field 1 | 1033593, 1088802 |
| Pairing identity | `bg_tachyon_pairing_attempt_id` is saved from the Gaia ceremony's pairing attempt | 994803, 1087261 |
| Received reactions | Message field 19; emoji object field 1 and repeated actor IDs field 2 | 1009621 |
| Received reply | Message field 21; reference field 1, with assigned/original ID fallbacks in nested field 6 | 1006865 |

Reaction actions are 1 add, 2 remove, and 3 replace. The native command currently
implements add and remove. It does not infer an existing user's reaction or
optimistically replace the displayed state. Received phone records supply
reaction counts and reply references.

Gaia unpairing uses the saved native pairing attempt ID. It does not use the
QR Phone Relay `RevokeRelayPairing` RPC or its unrelated registration source ID.

## Runtime behavior

Replies use the existing durable text-send operation, correlation, and uncertain
outcome handling. Reply and reaction controls are available on RCS conversations
once the phone's sending check passes. SMS and unknown conversations do not
advertise these controls.

Reactions run on the account's single live receive owner. One mutation per account
and eight across the helper are allowed. Correlated, authenticated phone responses
establish success; HTTP acceptance alone does not. Queued requests whose caller
has expired are skipped, and uncertain mutations are not automatically retried.
Receive recovery can reconnect without replaying a reaction.

For confirmed native accounts, Disconnect asks the phone to unpair first. A
positive action-46 result permits local credential cleanup. Rejection, timeout,
connection loss, or an offline account retains saved credentials. A later ACK
failure does not erase an already validated phone result. The UI shows completion
only after the daemon removes the account. Pending registrations without a
confirmed phone can still be forgotten locally.

## Validation

Wire fixtures cover reply targets, reaction add/remove, explicit result handling,
reply references, and reaction actors. The local encrypted mock phone exercises
reaction success, rejection, unpair confirmation, shared receive ownership, ACKs,
and expired queued requests. Helper tests cover reply admission, capability
serialization, and retaining offline confirmed records on a logout request.

Chrome and Helium passed the blank-page launch/private-pipe check. The user
confirmed Helium was used for the successful full setup, pairing, restart, and
local disconnect/reconnect checks. Official Chrome sign-in and read-only
authentication also passed earlier. Other browsers remain unverified.
