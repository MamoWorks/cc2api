use rustls::craft::{
    CraftExtension, ExtensionSpec, Fingerprint, GreaseOrCipher, GreaseOrCurve, GreaseOrVersion,
    KeepExtension,
};
use rustls::crypto::{ActiveKeyExchange, SharedSecret, SupportedKxGroup};
use rustls::internal::msgs::base::Payload;
use rustls::internal::msgs::enums::{ECPointFormat, ExtensionType, PSKKeyExchangeMode};
use rustls::internal::msgs::handshake::ClientExtension;
use rustls::{CipherSuite, Error, NamedGroup, ProtocolVersion, RootCertStore, SignatureScheme};
use static_init::dynamic;
use std::sync::Arc;
use std::time::Duration;

// ---------------------------------------------------------------------------
// X25519MLKEM768 混合密钥交换（真实实现）
// 按 draft-ietf-tls-ecdhe-mlkem：
//   client key_share = ML-KEM encaps key (1184) || X25519 pub (32) = 1216 bytes
//   server key_share = ML-KEM ciphertext (1088) || X25519 pub (32) = 1120 bytes
//   shared_secret    = ML-KEM shared secret (32) || X25519 shared (32) = 64 bytes
// ---------------------------------------------------------------------------
const X25519MLKEM768_GROUP: NamedGroup = NamedGroup::Unknown(0x11EC);

#[derive(Debug)]
struct X25519Mlkem768KxGroup;

impl SupportedKxGroup for X25519Mlkem768KxGroup {
    fn start(&self) -> Result<Box<dyn ActiveKeyExchange>, Error> {
        use ml_kem::{EncodedSizeUser, KemCore, MlKem768};

        let mut rng = rand::thread_rng();

        // ML-KEM-768 keypair
        let (dk, ek) = MlKem768::generate(&mut rng);
        let ek_bytes = &ek.as_bytes();

        // X25519 keypair
        let x25519_secret = x25519_dalek::StaticSecret::random_from_rng(&mut rng);
        let x25519_public = x25519_dalek::PublicKey::from(&x25519_secret);

        // client key_share = ek (1184) || x25519_pub (32)
        let mut pub_key = Vec::with_capacity(1216);
        pub_key.extend_from_slice(ek_bytes);
        pub_key.extend_from_slice(x25519_public.as_bytes());

        Ok(Box::new(X25519Mlkem768ActiveKx {
            dk,
            x25519_secret,
            pub_key,
        }))
    }

    fn name(&self) -> NamedGroup {
        X25519MLKEM768_GROUP
    }
}

struct X25519Mlkem768ActiveKx {
    dk: ml_kem::kem::DecapsulationKey<ml_kem::MlKem768Params>,
    x25519_secret: x25519_dalek::StaticSecret,
    pub_key: Vec<u8>,
}

impl std::fmt::Debug for X25519Mlkem768ActiveKx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("X25519Mlkem768ActiveKx").finish()
    }
}

impl ActiveKeyExchange for X25519Mlkem768ActiveKx {
    fn complete(self: Box<Self>, peer_pub_key: &[u8]) -> Result<SharedSecret, Error> {
        use ml_kem::kem::Decapsulate as _;

        // server key_share = ciphertext (1088) || x25519_pub (32) = 1120 bytes
        if peer_pub_key.len() != 1120 {
            return Err(Error::General(format!(
                "X25519MLKEM768: invalid server key_share length {}",
                peer_pub_key.len()
            )));
        }

        let (ct_bytes, x25519_peer) = peer_pub_key.split_at(1088);

        // ML-KEM decapsulation
        let ct: ml_kem::Ciphertext<ml_kem::MlKem768> = ct_bytes
            .try_into()
            .map_err(|_| Error::General("ML-KEM: invalid ciphertext".into()))?;
        let mlkem_ss = self
            .dk
            .decapsulate(&ct)
            .map_err(|_| Error::General("ML-KEM decapsulation failed".into()))?;

        // X25519 DH
        let x25519_peer_key: [u8; 32] = x25519_peer
            .try_into()
            .map_err(|_| Error::General("X25519: invalid peer key".into()))?;
        let x25519_peer_pub = x25519_dalek::PublicKey::from(x25519_peer_key);
        let x25519_ss = self.x25519_secret.diffie_hellman(&x25519_peer_pub);

        // shared_secret = mlkem_ss (32) || x25519_ss (32)
        let mut shared = Vec::with_capacity(64);
        shared.extend_from_slice(mlkem_ss.as_ref());
        shared.extend_from_slice(x25519_ss.as_bytes());

        Ok(SharedSecret::from(&shared[..]))
    }

    fn pub_key(&self) -> &[u8] {
        &self.pub_key
    }

    fn group(&self) -> NamedGroup {
        X25519MLKEM768_GROUP
    }
}

// X448 fake group（ring 不支持，只声明不使用）
#[derive(Debug)]
struct FakeKxGroup(NamedGroup);

impl SupportedKxGroup for FakeKxGroup {
    fn start(&self) -> Result<Box<dyn ActiveKeyExchange>, Error> {
        Err(Error::General(format!(
            "key exchange not supported for {:?}",
            self.0
        )))
    }
    fn name(&self) -> NamedGroup {
        self.0
    }
}

static X25519MLKEM768_KX: X25519Mlkem768KxGroup = X25519Mlkem768KxGroup;
static FAKE_X448: FakeKxGroup = FakeKxGroup(NamedGroup::Unknown(0x001E));

macro_rules! static_ref {
    ($val:expr, $type:ty) => {{
        static X: $type = $val;
        X
    }};
}

// ---------------------------------------------------------------------------
// Node.js 密码套件（52 个，与 tls.peet.ws 实测对齐）
// ---------------------------------------------------------------------------
#[dynamic]
pub static NODEJS_CIPHER: Vec<GreaseOrCipher> = vec![
    GreaseOrCipher::T(CipherSuite::TLS13_AES_256_GCM_SHA384),
    GreaseOrCipher::T(CipherSuite::TLS13_CHACHA20_POLY1305_SHA256),
    GreaseOrCipher::T(CipherSuite::TLS13_AES_128_GCM_SHA256),
    GreaseOrCipher::T(CipherSuite::Unknown(0xC02F)),
    GreaseOrCipher::T(CipherSuite::Unknown(0xC02B)),
    GreaseOrCipher::T(CipherSuite::Unknown(0xC030)),
    GreaseOrCipher::T(CipherSuite::Unknown(0xC02C)),
    GreaseOrCipher::T(CipherSuite::Unknown(0x009E)),
    GreaseOrCipher::T(CipherSuite::Unknown(0xC027)),
    GreaseOrCipher::T(CipherSuite::Unknown(0x0067)),
    GreaseOrCipher::T(CipherSuite::Unknown(0xC028)),
    GreaseOrCipher::T(CipherSuite::Unknown(0x006B)),
    GreaseOrCipher::T(CipherSuite::Unknown(0x00A3)),
    GreaseOrCipher::T(CipherSuite::Unknown(0x009F)),
    GreaseOrCipher::T(CipherSuite::Unknown(0xCCA9)),
    GreaseOrCipher::T(CipherSuite::Unknown(0xCCA8)),
    GreaseOrCipher::T(CipherSuite::Unknown(0xCCAA)),
    GreaseOrCipher::T(CipherSuite::Unknown(0xC0AD)),
    GreaseOrCipher::T(CipherSuite::Unknown(0xC09F)),
    GreaseOrCipher::T(CipherSuite::Unknown(0xC05D)),
    GreaseOrCipher::T(CipherSuite::Unknown(0xC061)),
    GreaseOrCipher::T(CipherSuite::Unknown(0xC057)),
    GreaseOrCipher::T(CipherSuite::Unknown(0xC053)),
    GreaseOrCipher::T(CipherSuite::Unknown(0x00A2)),
    GreaseOrCipher::T(CipherSuite::Unknown(0xC0AC)),
    GreaseOrCipher::T(CipherSuite::Unknown(0xC09E)),
    GreaseOrCipher::T(CipherSuite::Unknown(0xC05C)),
    GreaseOrCipher::T(CipherSuite::Unknown(0xC060)),
    GreaseOrCipher::T(CipherSuite::Unknown(0xC056)),
    GreaseOrCipher::T(CipherSuite::Unknown(0xC052)),
    GreaseOrCipher::T(CipherSuite::Unknown(0xC024)),
    GreaseOrCipher::T(CipherSuite::Unknown(0x006A)),
    GreaseOrCipher::T(CipherSuite::Unknown(0xC023)),
    GreaseOrCipher::T(CipherSuite::Unknown(0x0040)),
    GreaseOrCipher::T(CipherSuite::Unknown(0xC00A)),
    GreaseOrCipher::T(CipherSuite::Unknown(0xC014)),
    GreaseOrCipher::T(CipherSuite::Unknown(0x0039)),
    GreaseOrCipher::T(CipherSuite::Unknown(0x0038)),
    GreaseOrCipher::T(CipherSuite::Unknown(0xC009)),
    GreaseOrCipher::T(CipherSuite::Unknown(0xC013)),
    GreaseOrCipher::T(CipherSuite::Unknown(0x0033)),
    GreaseOrCipher::T(CipherSuite::Unknown(0x0032)),
    GreaseOrCipher::T(CipherSuite::Unknown(0x009D)),
    GreaseOrCipher::T(CipherSuite::Unknown(0xC09D)),
    GreaseOrCipher::T(CipherSuite::Unknown(0xC051)),
    GreaseOrCipher::T(CipherSuite::Unknown(0x009C)),
    GreaseOrCipher::T(CipherSuite::Unknown(0xC09C)),
    GreaseOrCipher::T(CipherSuite::Unknown(0xC050)),
    GreaseOrCipher::T(CipherSuite::Unknown(0x003D)),
    GreaseOrCipher::T(CipherSuite::Unknown(0x003C)),
    GreaseOrCipher::T(CipherSuite::Unknown(0x0035)),
    GreaseOrCipher::T(CipherSuite::Unknown(0x002F)),
];

// ---------------------------------------------------------------------------
// Node.js 扩展列表（12 个，精确顺序匹配 tls.peet.ws 实测）
// ---------------------------------------------------------------------------
#[dynamic]
pub static NODEJS_EXTENSION: Vec<ExtensionSpec> = {
    use ExtensionSpec::*;
    use KeepExtension::*;
    vec![
        // 1. renegotiation_info (65281)
        Craft(CraftExtension::RenegotiationInfo),
        // 2. server_name (0)
        Keep(Must(ExtensionType::ServerName)),
        // 3. ec_point_formats (11)
        Rustls(ClientExtension::EcPointFormats(vec![
            ECPointFormat::Uncompressed,
            ECPointFormat::ANSIX962CompressedPrime,
            ECPointFormat::ANSIX962CompressedChar2,
        ])),
        // 4. supported_groups (10)
        Rustls(ClientExtension::NamedGroups(vec![
            NamedGroup::Unknown(0x11EC), // X25519MLKEM768
            NamedGroup::X25519,
            NamedGroup::secp256r1,
            NamedGroup::Unknown(0x001E), // X448
            NamedGroup::secp384r1,
            NamedGroup::secp521r1,
            NamedGroup::FFDHE2048,
            NamedGroup::FFDHE3072,
        ])),
        // 5. session_ticket (35)
        Keep(OrDefault(
            ExtensionType::SessionTicket,
            ClientExtension::SessionTicket(
                rustls::internal::msgs::handshake::ClientSessionTicket::Offer(Payload(vec![])),
            ),
        )),
        // 6. ALPN (16)
        Craft(CraftExtension::Protocols(&[b"http/1.1"])),
        // 7. encrypt_then_mac (22)
        Rustls(ClientExtension::Unknown(
            rustls::internal::msgs::handshake::UnknownExtension {
                typ: ExtensionType::Unknown(22),
                payload: Payload(vec![]),
            },
        )),
        // 8. extended_master_secret (23)
        Rustls(ClientExtension::ExtendedMasterSecretRequest),
        // 9. signature_algorithms (13)
        Rustls(ClientExtension::SignatureAlgorithms(vec![
            SignatureScheme::Unknown(0x0905),
            SignatureScheme::Unknown(0x0906),
            SignatureScheme::Unknown(0x0904),
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::Unknown(0x0603),
            SignatureScheme::Unknown(0x0807),
            SignatureScheme::Unknown(0x0808),
            SignatureScheme::Unknown(0x081a),
            SignatureScheme::Unknown(0x081b),
            SignatureScheme::Unknown(0x081c),
            SignatureScheme::Unknown(0x0809),
            SignatureScheme::Unknown(0x080a),
            SignatureScheme::Unknown(0x080b),
            SignatureScheme::Unknown(0x0804),
            SignatureScheme::Unknown(0x0805),
            SignatureScheme::Unknown(0x0806),
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::Unknown(0x0303),
            SignatureScheme::Unknown(0x0301),
            SignatureScheme::Unknown(0x0302),
            SignatureScheme::Unknown(0x0402),
            SignatureScheme::Unknown(0x0502),
            SignatureScheme::Unknown(0x0602),
        ])),
        // 10. supported_versions (43)
        Craft(CraftExtension::SupportedVersions(static_ref!(
            &[
                GreaseOrVersion::T(ProtocolVersion::TLSv1_3),
                GreaseOrVersion::T(ProtocolVersion::TLSv1_2),
            ],
            &[GreaseOrVersion]
        ))),
        // 11. psk_key_exchange_modes (45)
        Rustls(ClientExtension::PresharedKeyModes(vec![
            PSKKeyExchangeMode::PSK_DHE_KE,
        ])),
        // 12. key_share (51)
        Craft(CraftExtension::KeyShare(&[GreaseOrCurve::T(
            NamedGroup::X25519,
        )])),
    ]
};

#[dynamic]
pub static NODEJS_FINGERPRINT: Fingerprint = Fingerprint {
    extensions: &NODEJS_EXTENSION,
    cipher: &NODEJS_CIPHER,
    shuffle_extensions: false,
};

/// 构建带 Node.js TLS 指纹的 rustls ClientConfig。
fn build_tls_config() -> rustls::ClientConfig {
    let root_store = RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };

    let mut config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth()
        .with_fingerprint(NODEJS_FINGERPRINT.builder());

    // 将 supported_groups 中声明但 ring 不支持的 group 注册为 fake KxGroup，
    // 确保 HRR 验证时 find_kx_group() 能找到它们。
    let mut provider = config.provider.as_ref().clone();
    provider.kx_groups.insert(0, &X25519MLKEM768_KX);
    provider.kx_groups.push(&FAKE_X448);
    config.provider = Arc::new(provider);

    config
}

/// 创建带 TLS 指纹伪装的 reqwest 客户端。
/// 支持直连和代理（HTTP/SOCKS5）。
pub fn make_request_client(proxy_url: &str) -> reqwest::Client {
    let tls_config = build_tls_config();

    let mut builder = reqwest::Client::builder()
        .use_preconfigured_tls(tls_config)
        .timeout(Duration::from_secs(300))
        .no_proxy();

    if !proxy_url.is_empty() {
        if let Ok(proxy) = reqwest::Proxy::all(proxy_url) {
            builder = builder.proxy(proxy);
        }
    }

    builder.build().unwrap_or_else(|_| reqwest::Client::new())
}
