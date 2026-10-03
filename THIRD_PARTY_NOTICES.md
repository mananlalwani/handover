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
