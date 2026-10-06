use anyhow::{bail, Result};
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
    DigitallySignedStruct, SignatureScheme,
};
use sha2::{Digest, Sha256};
use std::{io::BufReader, os::unix::fs::OpenOptionsExt, path::Path, sync::Arc};

pub fn identity(dir: &Path) -> Result<(Vec<u8>, Vec<u8>, String)> {
    std::fs::create_dir_all(dir)?;
    let cert_path = dir.join("localsend-cert.pem");
    let key_path = dir.join("localsend-key.pem");
    if !cert_path.exists() && !key_path.exists() {
        let cert = rcgen::generate_simple_self_signed(vec!["linuxdrop.local".into()])?;
        for (path, content) in [
            (&key_path, cert.key_pair.serialize_pem()),
            (&cert_path, cert.cert.pem()),
        ] {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)?;
            file.write_all(content.as_bytes())?;
            file.sync_all()?;
        }
    }
    let cert = std::fs::read(cert_path)?;
    let key = std::fs::read(key_path)?;
    let der = rustls_pemfile::certs(&mut BufReader::new(cert.as_slice()))
        .next()
        .transpose()?
        .ok_or_else(|| anyhow::anyhow!("Certificate missing"))?;
    let fingerprint = hex::encode(Sha256::digest(der.as_ref()));
    Ok((cert, key, fingerprint))
}

#[derive(Debug)]
struct CertificatePin {
    fingerprint: String,
}
impl ServerCertVerifier for CertificatePin {
    fn verify_server_cert(
        &self,
        cert: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if hex::encode(Sha256::digest(cert.as_ref())) != self.fingerprint {
            return Err(rustls::Error::General(
                "LocalSend certificate fingerprint mismatch".into(),
            ));
        }
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            signature,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            signature,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(test)]
pub fn client(protocol: &str, fingerprint: &str) -> Result<reqwest::Client> {
    client_on(protocol, fingerprint, None)
}

pub fn client_on(
    protocol: &str,
    fingerprint: &str,
    interface: Option<&str>,
) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy();
    if let Some(interface) = interface {
        builder = builder.interface(interface);
    }
    if protocol == "http" {
        return Ok(builder.build()?);
    }
    let fingerprint = fingerprint.to_ascii_lowercase().replace(':', "");
    if fingerprint.len() != 64 || !fingerprint.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("Invalid HTTPS fingerprint");
    }
    let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .dangerous()
    .with_custom_certificate_verifier(Arc::new(CertificatePin { fingerprint }))
    .with_no_client_auth();
    Ok(builder.use_preconfigured_tls(tls).build()?)
}
