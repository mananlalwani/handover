// Copyright 2025 Google LLC
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

use tinyvec::ArrayVec;

/// The maximum length of a DER-encoded ecdsa `Signature`, in bytes.
pub const SIGNATURE_MAX_LENGTH: usize = 72;

/// The length of an ecdsa `PrivateKey`, in bytes.
pub const PRIVATE_KEY_LENGTH: usize = 32;

/// The length of a sec1 ecdsa `PublicKey`, in bytes.
pub const PUBLIC_KEY_LENGTH: usize = 65;

/// A byte buffer the size of a P256 ecdsa `Signature`.
pub type RawP256Signature = ArrayVec<[u8; SIGNATURE_MAX_LENGTH]>;

/// A byte buffer the size of a ecdsa `PublicKey`.
pub type RawP256PublicKey = [u8; PUBLIC_KEY_LENGTH];

/// A byte buffer the size of a ecdsa `PrivateKey`.
pub type RawP256PrivateKey = [u8; PRIVATE_KEY_LENGTH];

/// A permission token which may be supplied to methods which allow
/// converting private keys to/from raw bytes.
///
/// In general, operations of this kind should only be done in
/// development-tools, tests, or in credential storage layers
/// to prevent accidental exposure of the private key.
pub struct RawPrivateKeyPermit {
    _marker: (),
}

impl RawPrivateKeyPermit {
    /// Allows for internal construction of a private key permit to
    /// allow accessing a raw private key.
    fn new() -> Self {
        Self { _marker: () }
    }
}

/// This implementation is behind a feature flag to allow auditing of uses for
/// constructing a private key from raw bytes.
#[cfg(feature = "raw_private_key_permit")]
impl core::default::Default for RawPrivateKeyPermit {
    fn default() -> Self {
        Self::new()
    }
}

/// A crypto-provider-independent representation of the private
/// key of a P256 ECDSA key-pair, kept in such a way that
/// it does not permit de-structuring it into raw bytes,
/// nor constructing one from raw bytes.
pub struct P256PrivateKey {
    key: RawP256PrivateKey,
}

impl P256PrivateKey {
    /// This must be kept private in order to ensure safety of the private key material.
    /// [Self::from_raw_private_key] adds a [RawPrivateKeyPermit] to allow auditing of usages to
    /// ensure that proper precautions are taken when handling the raw key material.
    fn new(key: RawP256PrivateKey) -> Self {
        Self { key }
    }

    /// Derives the public key corresponding to this private key.
    pub fn derive_public_key<E: P256EcdsaProvider>(&self) -> impl PublicKey {
        let key_pair = E::P256KeyPair::from_private_key(self);
        key_pair.public_key()
    }

    /// Sign the given message and return a digital signature
    pub fn sign<E: P256EcdsaProvider>(&self, msg: &[u8]) -> impl Signature {
        let key_pair = E::P256KeyPair::from_private_key(self);
        key_pair.sign(msg)
    }

    /// Generate an ecdsa private key from a CSPRNG
    /// generate is not available in `no-std`.
    #[cfg(feature = "std")]
    pub fn generate<E: P256EcdsaProvider>() -> Self {
        let key_pair = E::P256KeyPair::generate();
        (&key_pair).into()
    }

    /// Returns the raw bytes of this private key.
    /// This operation is only possible while holding a [`RawPrivateKeyPermit`].
    pub fn raw_private_key(&self, _permit: &RawPrivateKeyPermit) -> RawP256PrivateKey {
        self.key
    }

    /// Constructs a private key from the raw bytes of the key.
    /// This operation is only possible while holding a [`RawPrivateKeyPermit`].
    pub fn from_raw_private_key(wrapped: RawP256PrivateKey, _permit: &RawPrivateKeyPermit) -> Self {
        P256PrivateKey::new(wrapped)
    }
}

/// Error returned when bad bytes are used to instantiate a private key.
#[derive(Debug)]
pub struct InvalidPrivateKeyBytes;

/// error returned when bad bytes are provided to generate keypair
#[derive(Debug)]
pub struct InvalidPublicKeyBytes;

/// Error returned invalid bytes are used to instantiate a signature.
#[derive(Debug)]
pub struct InvalidSignatureBytes;

/// Error returned if the verification on the signature + message fails
#[derive(Debug)]
pub struct VerifyError;

/// Collection of types used to provide an implementation of ecdsa.
pub trait P256EcdsaProvider {
    /// The internal representation of a keypair which includes both public and secret halves of an asymmetric key.
    type P256KeyPair: KeyPair<
        P256PublicKey = Self::P256PublicKey,
        P256Signature = Self::P256Signature,
    >;
    /// The internal representation of an ecdsa public key, used when verifying a message
    type P256PublicKey: PublicKey<Signature = Self::P256Signature>;
    /// The internal representation of an ecdsa signature which is the result of signing a message
    type P256Signature: Signature;
}

/// The keypair which includes both public and secret halves of an asymmetric key.
/// This is different from [Self::P256PublicKey] as it includes operations to create a raw private
/// key as well. The private key operations are contained in this trait, and the public key can be
/// extracted from the private key to be used for verification. This trait is what providers
/// implement to add ECDSA-P256 functionality.
pub trait KeyPair: Sized {
    /// The ecdsa public key, used when verifying a message
    type P256PublicKey: PublicKey;

    /// The ecdsa signature returned when signing a message
    type P256Signature: Signature;

    /// Returns the private key bytes of the `KeyPair`.
    /// This operation is only possible while holding a [`RawPrivateKeyPermit`].
    fn raw_private_key(&self, _permit: &RawPrivateKeyPermit) -> RawP256PrivateKey;

    /// Builds a key-pair from a `RawPrivateKey` array of bytes.
    /// This operation is only possible while holding a [`RawPrivateKeyPermit`].
    fn from_raw_private_key(bytes: &RawP256PrivateKey, _permit: &RawPrivateKeyPermit) -> Self
    where
        Self: Sized;

    /// Sign the given message and return a digital signature
    fn sign(&self, msg: &[u8]) -> Self::P256Signature;

    /// Generate an ecdsa keypair from a CSPRNG
    /// generate is not available in `no-std`
    #[cfg(feature = "std")]
    fn generate() -> Self;

    /// Builds a key-pair from a [`P256PrivateKey`], given in an opaque form.
    fn from_private_key(private_key: &P256PrivateKey) -> Self
    where
        Self: Sized,
    {
        // We're okay to reach in and construct an instance from
        // the bytes of the private key, since the way that they
        // were originally expressed would still require a valid
        // [`RawPrivateKeyPermit`] to access them.
        let raw_private_key = &private_key.key;
        Self::from_raw_private_key(raw_private_key, &RawPrivateKeyPermit::new())
    }

    /// Getter function for the Public Key of the key pair
    fn public_key(&self) -> Self::P256PublicKey;
}

impl<K> From<&K> for P256PrivateKey
where
    K: KeyPair,
{
    fn from(value: &K) -> Self {
        // We're okay to reach in and grab the bytes of the private key,
        // since the way that we're exposing it would require a valid
        // [`RawPrivateKeyPermit`] to extract them again.
        let wrapped = value.raw_private_key(&RawPrivateKeyPermit::new());
        P256PrivateKey::new(wrapped)
    }
}

/// An ecdsa signature
pub trait Signature: Sized {
    /// Create a new signature from a fixed size byte array. This represents a container for the
    /// byte serialization of an ECDSA signature, and does not necessarily represent well-formed
    /// field or curve elements.
    ///
    /// Signature verification libraries are expected to reject invalid field
    /// elements at the time a signature is verified (not constructed).
    fn from_bytes(bytes: &RawP256Signature) -> Result<Self, InvalidSignatureBytes>;

    /// Returns a slice of the signature bytes
    fn to_bytes(&self) -> RawP256Signature;
}

/// An ecdsa public key.
pub trait PublicKey {
    /// The signature type being used by verify
    type Signature: Signature;

    /// Builds this public key from an array of bytes in
    /// the SEC1 format.
    fn from_bytes(bytes: &RawP256PublicKey) -> Result<Self, InvalidPublicKeyBytes>
    where
        Self: Sized;

    /// Yields the bytes of the public key in a SEC1 format.
    fn to_bytes(&self) -> RawP256PublicKey;

    /// Succeeds if the signature was a valid signature created by this Keypair on the prehashed_message.
    fn verify_strict(&self, message: &[u8], signature: &Self::Signature)
        -> Result<(), VerifyError>;
}
