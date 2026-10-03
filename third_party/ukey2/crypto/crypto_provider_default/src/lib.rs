// Modified by Handover: expose only the included RustCrypto provider.
// Copyright 2023 Google LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Provides multiple implementations of CryptoProvider through the same struct, configurable by
//! feature flag.

#![no_std]

#[cfg(feature = "rustcrypto")]
pub use crypto_provider_rustcrypto::RustCrypto as CryptoProviderImpl;

#[cfg(not(feature = "rustcrypto"))]
compile_error!("The included provider selector requires the rustcrypto feature");
