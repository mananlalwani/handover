# Third-party notices

Handover's original code is licensed under MIT, as stated in [LICENSE](LICENSE).
Included third-party code retains its own license.

## Google UKEY2

The independent Google Messages client includes UKEY2 and its RustCrypto provider
from Google beto-core, revision `479289ef072b0880c0347d36937265e44f00f4ee`.
Copyright 2023 Google LLC. Licensed under Apache License, Version 2.0.

The license text is included in [third_party/ukey2/LICENSE](third_party/ukey2/LICENSE).
Upstream copyright headers remain in the source files. Modified files carry
Handover change notices; [third_party/ukey2/README.md](third_party/ukey2/README.md)
describes packaging and runtime changes. Redistributed copies containing this
code must retain the Apache license and applicable notices.

Registry dependencies retain the licenses declared in their packages. Handover's
cargo-deny policy checks the resolved dependency graph for permitted licenses.

## WebRTC audio

The Android app includes `io.github.webrtc-sdk:android:150.7871.01`.
The SDK packaging is MIT licensed, copyright 2023 WebRTC SDKs. Google WebRTC
and its bundled dependencies retain their own licenses. The
distributor's notices are included unchanged in
[android/app/src/main/assets/licenses](android/app/src/main/assets/licenses)
and packaged in the APK. Upstream notices come from the
[v150.7871.01 source tree](https://github.com/webrtc-sdk/android/tree/v150.7871.01/Licenses).

The Linux sender uses the MIT/Apache-2.0 `webrtc` and `opus` crates. Libopus
is BSD licensed; registry packages retain their license files.
