//! Timestamps from a timestamp authority (RFC 3161): proof, signed by someone outside of Bullseye,
//! that something existed at a time. The sync server has each entry of its log timestamped as it
//! comes in (`hub.rs`); a computer that was off or asleep then applies the entry as of that time
//! (`handler::apply`), as it would have, had it been on. Nobody with the server can make a token:
//! it needs the authority's key, and the authority signs only its own present time.
//!
//! A token is a CMS SignedData of a TSTInfo: the SHA-256 of what is timestamped and the time. It is
//! believed when the TSTInfo names that digest, the signature over it is the signer certificate's,
//! that certificate is for timestamping, and its chain leads to a pinned root key (`ROOTS`), each
//! certificate valid at the time. Revocation is not checked (a computer may be offline when it
//! applies an entry). What cannot be read, or anything else, is not a timestamp: the entry is then
//! applied when it arrives, as it always was.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::datetime::DateTime;

/// The timestamp authority the sync server asks: DigiCert's, free, plain HTTP (it answers no HTTPS;
/// the token is signed, so the connection need not be).
#[allow(dead_code)] // the sync server's (`server/src/main.rs`)
pub const URL: &str = "http://timestamp.digicert.com";

/// The keys a token's chain must lead to (SubjectPublicKeyInfo, DER): DigiCert Trusted Root G4, the
/// root of DigiCert's timestamping certificates.
const ROOTS: &[&[u8]] = &[include_bytes!("digicert-trusted-root-g4.der")];

/// How long the sync server waits for the authority.
#[cfg_attr(not(test), allow(dead_code))] // the sync server's
const TIMEOUT: Duration = Duration::from_secs(5);

/// The largest answer the sync server takes from the authority.
#[cfg_attr(not(test), allow(dead_code))] // the sync server's
const MAX_RESPONSE: u64 = 64 * 1024;

/// How many certificates up a chain may go.
const MAX_CHAIN: usize = 6;

// The object identifiers this reads, as their DER contents.
const SIGNED_DATA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x07, 0x02];
const TST_INFO: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x09, 0x10, 0x01, 0x04];
const CONTENT_TYPE: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x09, 0x03];
const MESSAGE_DIGEST: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x09, 0x04];
const RSA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01];
const RSA_SHA256: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0b];
const RSA_SHA384: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0c];
const RSA_SHA512: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0d];
const SHA256: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01];
const SHA384: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x02];
const SHA512: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x03];
const EXTENDED_KEY_USAGE: &[u8] = &[0x55, 0x1d, 0x25];
const SUBJECT_KEY_ID: &[u8] = &[0x55, 0x1d, 0x0e];
const TIME_STAMPING: &[u8] = &[0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x08];

// DER tags.
const BOOLEAN: u8 = 0x01;
const INTEGER: u8 = 0x02;
const BIT_STRING: u8 = 0x03;
const OCTET_STRING: u8 = 0x04;
#[cfg_attr(not(test), allow(dead_code))] // the sync server's
const NULL: u8 = 0x05;
const OID: u8 = 0x06;
const UTC_TIME: u8 = 0x17;
const GENERALIZED_TIME: u8 = 0x18;
const SEQUENCE: u8 = 0x30;
const SET: u8 = 0x31;
/// `[n]`, constructed.
const fn tagged(n: u8) -> u8 {
    0xa0 | n
}

/// The SHA-256 of `data`, as a token names it.
pub fn digest(data: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(data).into()
}

/// When the token says `digest` existed, if it is a token of a pinned authority for it. Tests also
/// pin their own authority (`tests::make`), never a build that is not a test.
pub fn verify(token: &[u8], digest: &[u8; 32]) -> Result<SystemTime, String> {
    #[cfg(test)]
    let roots = [ROOTS, &[tests::TEST_ROOT]].concat();
    #[cfg(not(test))]
    let roots = ROOTS.to_vec();
    verify_with(token, digest, &roots)
}

fn verify_with(token: &[u8], digest: &[u8; 32], roots: &[&[u8]]) -> Result<SystemTime, String> {
    let bad = |what: &str| format!("Not a timestamp: {what}");
    // ContentInfo: the type, and the SignedData.
    let mut info = Reader::new(token).sequence().ok_or_else(|| bad("no content"))?;
    if info.oid() != Some(SIGNED_DATA) {
        return Err(bad("not signed data"));
    }
    let mut signed = info.explicit(0).and_then(|mut c| c.sequence()).ok_or_else(|| bad("no signed data"))?;
    signed.next_of(INTEGER).ok_or_else(|| bad("no version"))?;
    signed.next_of(SET).ok_or_else(|| bad("no digest algorithms"))?;
    // What is signed: the TSTInfo.
    let mut content = signed.sequence().ok_or_else(|| bad("no content info"))?;
    if content.oid() != Some(TST_INFO) {
        return Err(bad("not a timestamp's content"));
    }
    let tst = content.explicit(0).and_then(|mut c| c.next_of(OCTET_STRING)).ok_or_else(|| bad("no timestamp info"))?;
    let at = tst_info(tst, digest)?;
    // The certificates that came with it, then exactly one signer.
    let mut certificates = Vec::new();
    if let Some(mut bag) = signed.optional(tagged(0)) {
        while let Some(certificate) = bag.raw_of(SEQUENCE) {
            certificates.push(Certificate::read(certificate).ok_or_else(|| bad("a certificate cannot be read"))?);
        }
    }
    signed.optional(tagged(1));
    let mut signers = signed.next_of(SET).map(Reader::new).ok_or_else(|| bad("no signer"))?;
    let signer = signers.sequence().ok_or_else(|| bad("no signer"))?;
    if !signers.done() {
        return Err(bad("more than one signer"));
    }
    let certificate = signer_info(signer, tst, &certificates)?;
    if !certificate.stamps() {
        return Err(bad("its signer is not for timestamping"));
    }
    chain(certificate, &certificates, roots, at)?;
    Ok(at)
}

/// A TSTInfo's time, if it names `digest` (SHA-256).
fn tst_info(tst: &[u8], digest: &[u8; 32]) -> Result<SystemTime, String> {
    let bad = |what: &str| format!("Not a timestamp: {what}");
    let mut info = Reader::new(tst).sequence().ok_or_else(|| bad("no timestamp info"))?;
    info.next_of(INTEGER).ok_or_else(|| bad("no version"))?;
    info.oid().ok_or_else(|| bad("no policy"))?;
    let mut imprint = info.sequence().ok_or_else(|| bad("no digest"))?;
    let algorithm = imprint.sequence().and_then(|mut a| a.oid()).ok_or_else(|| bad("no digest algorithm"))?;
    let hashed = imprint.next_of(OCTET_STRING).ok_or_else(|| bad("no digest"))?;
    if algorithm != SHA256 || hashed != digest {
        return Err(bad("of something else"));
    }
    info.next_of(INTEGER).ok_or_else(|| bad("no serial number"))?;
    let time = info.next_of(GENERALIZED_TIME).ok_or_else(|| bad("no time"))?;
    time_of(GENERALIZED_TIME, time).ok_or_else(|| bad("its time cannot be read"))
}

/// Checks a SignerInfo: its signed attributes name the TSTInfo `tst`, and its certificate (among
/// `certificates`) signed them. That certificate.
fn signer_info<'a>(mut signer: Reader<'a>, tst: &[u8], certificates: &'a [Certificate<'a>]) -> Result<&'a Certificate<'a>, String> {
    let bad = |what: &str| format!("Not a timestamp: {what}");
    signer.next_of(INTEGER).ok_or_else(|| bad("no signer version"))?;
    // Which certificate: by issuer and serial number, or by subject key identifier.
    let certificate = match signer.peek() {
        Some(SEQUENCE) => {
            let mut id = signer.sequence().ok_or_else(|| bad("no signer"))?;
            let issuer = id.raw_of(SEQUENCE).ok_or_else(|| bad("no signer's issuer"))?;
            let serial = id.next_of(INTEGER).ok_or_else(|| bad("no signer's serial number"))?;
            certificates.iter().find(|c| c.issuer == issuer && c.serial == serial)
        }
        Some(0x80) => {
            let key_id = signer.next_of(0x80).ok_or_else(|| bad("no signer"))?;
            certificates.iter().find(|c| c.key_id == Some(key_id))
        }
        _ => None,
    }
    .ok_or_else(|| bad("its signer's certificate is missing"))?;
    let digest = signer.sequence().and_then(|mut a| a.oid()).ok_or_else(|| bad("no digest algorithm"))?;
    let attributes = signer.raw_of(tagged(0)).ok_or_else(|| bad("no signed attributes"))?;
    let algorithm = signer.sequence().and_then(|mut a| a.oid()).ok_or_else(|| bad("no signature algorithm"))?;
    let signature = signer.next_of(OCTET_STRING).ok_or_else(|| bad("no signature"))?;
    // The attributes name a TSTInfo with the digest of `tst`.
    let mut content_type = None;
    let mut message_digest = None;
    let mut list = Reader::new(attributes).next_of(tagged(0)).map(Reader::new).ok_or_else(|| bad("no signed attributes"))?;
    while let Some(mut attribute) = list.sequence() {
        let kind = attribute.oid().ok_or_else(|| bad("an attribute has no type"))?;
        let mut values = attribute.next_of(SET).map(Reader::new).ok_or_else(|| bad("an attribute has no value"))?;
        if kind == CONTENT_TYPE {
            content_type = values.oid();
        } else if kind == MESSAGE_DIGEST {
            message_digest = values.next_of(OCTET_STRING);
        }
    }
    if !list.done() {
        return Err(bad("its signed attributes cannot be read"));
    }
    if content_type != Some(TST_INFO) {
        return Err(bad("it signs something else"));
    }
    if message_digest != Some(&hash(digest, tst).ok_or_else(|| bad("an unknown digest algorithm"))?[..]) {
        return Err(bad("it signs other timestamp info"));
    }
    // Signed as a SET, which is what the [0] stands for.
    let mut signed = attributes.to_vec();
    signed[0] = SET;
    let algorithm = if algorithm == RSA { signature_of(digest) } else { Some(algorithm) };
    if !verify_rsa(algorithm.ok_or_else(|| bad("an unknown signature algorithm"))?, certificate.key, &signed, signature) {
        return Err(bad("its signature is not its signer's"));
    }
    Ok(certificate)
}

/// Follows `certificate` up through `certificates` to a key of `roots`, each valid at `at`.
fn chain<'a>(
    mut certificate: &'a Certificate<'a>,
    certificates: &'a [Certificate<'a>],
    roots: &[&[u8]],
    at: SystemTime,
) -> Result<(), String> {
    for _ in 0..MAX_CHAIN {
        if !certificate.valid_at(at) {
            return Err("Not a timestamp: a certificate of it was not valid then".into());
        }
        if roots.iter().any(|root| certificate.signed_by(root)) {
            return Ok(());
        }
        // A certificate that signed itself and is not pinned ends it: a root nobody knows.
        certificate = certificates
            .iter()
            .filter(|issuer| !std::ptr::eq(*issuer, certificate))
            .find(|issuer| issuer.subject == certificate.issuer && certificate.signed_by(issuer.key_info))
            .ok_or("Not a timestamp: it does not lead to a known authority")?;
    }
    Err("Not a timestamp: its chain is too long".into())
}

/// What is read of an X.509 certificate.
struct Certificate<'a> {
    tbs: &'a [u8],
    algorithm: &'a [u8],
    signature: &'a [u8],
    serial: &'a [u8],
    issuer: &'a [u8],
    subject: &'a [u8],
    not_before: SystemTime,
    not_after: SystemTime,
    /// The SubjectPublicKeyInfo, and its RSA key (`None` for another kind).
    key_info: &'a [u8],
    key: &'a [u8],
    key_id: Option<&'a [u8]>,
    /// Extended key usage: whether it is marked critical, and its purposes.
    usage: Option<(bool, Vec<&'a [u8]>)>,
}

impl<'a> Certificate<'a> {
    fn read(der: &'a [u8]) -> Option<Certificate<'a>> {
        let mut certificate = Reader::new(der).sequence()?;
        let tbs = certificate.raw_of(SEQUENCE)?;
        let algorithm = certificate.sequence()?.oid()?;
        let signature = bits(certificate.next_of(BIT_STRING)?)?;
        let mut fields = Reader::new(tbs).sequence()?;
        fields.optional(tagged(0));
        let serial = fields.next_of(INTEGER)?;
        fields.raw_of(SEQUENCE)?;
        let issuer = fields.raw_of(SEQUENCE)?;
        let mut validity = fields.sequence()?;
        let mut time = || {
            let tag = validity.peek()?;
            time_of(tag, validity.next_of(tag)?)
        };
        let (not_before, not_after) = (time()?, time()?);
        let subject = fields.raw_of(SEQUENCE)?;
        let key_info = fields.raw_of(SEQUENCE)?;
        let mut spki = Reader::new(key_info).sequence()?;
        let rsa = spki.sequence()?.oid()? == RSA;
        let key = if rsa { bits(spki.next_of(BIT_STRING)?)? } else { &[] };
        fields.optional(tagged(1));
        fields.optional(tagged(2));
        let (mut key_id, mut usage) = (None, None);
        if let Some(mut extensions) = fields.explicit(3).and_then(|mut e| e.sequence()) {
            while let Some(mut extension) = extensions.sequence() {
                let id = extension.oid()?;
                let critical = extension.peek() == Some(BOOLEAN) && extension.next_of(BOOLEAN)? != [0];
                let value = extension.next_of(OCTET_STRING)?;
                if id == SUBJECT_KEY_ID {
                    key_id = Some(Reader::new(value).next_of(OCTET_STRING)?);
                } else if id == EXTENDED_KEY_USAGE {
                    let mut purposes = Reader::new(value).sequence()?;
                    let mut list = Vec::new();
                    while let Some(purpose) = purposes.oid() {
                        list.push(purpose);
                    }
                    usage = Some((critical, list));
                }
            }
        }
        Some(Certificate { tbs, algorithm, signature, serial, issuer, subject, not_before, not_after, key_info, key, key_id, usage })
    }

    /// For timestamps: its extended key usage is critical and is timestamping (RFC 3161, 2.3).
    fn stamps(&self) -> bool {
        matches!(&self.usage, Some((true, purposes)) if purposes.as_slice() == [TIME_STAMPING])
    }

    fn valid_at(&self, at: SystemTime) -> bool {
        self.not_before <= at && at <= self.not_after
    }

    /// Whether the key `key_info` (a SubjectPublicKeyInfo) signed this certificate.
    fn signed_by(&self, key_info: &[u8]) -> bool {
        let Some(mut spki) = Reader::new(key_info).sequence() else { return false };
        if spki.sequence().and_then(|mut a| a.oid()) != Some(RSA) {
            return false;
        }
        let Some(key) = spki.next_of(BIT_STRING).and_then(bits) else { return false };
        verify_rsa(self.algorithm, key, self.tbs, self.signature)
    }
}

/// An RSA PKCS #1 v1.5 signature, by its algorithm (SHA-256, -384 or -512 with RSA).
fn verify_rsa(algorithm: &[u8], key: &[u8], message: &[u8], signature: &[u8]) -> bool {
    use ring::signature::{UnparsedPublicKey, RSA_PKCS1_2048_8192_SHA256, RSA_PKCS1_2048_8192_SHA384, RSA_PKCS1_2048_8192_SHA512};
    let scheme = match algorithm {
        RSA_SHA256 => &RSA_PKCS1_2048_8192_SHA256,
        RSA_SHA384 => &RSA_PKCS1_2048_8192_SHA384,
        RSA_SHA512 => &RSA_PKCS1_2048_8192_SHA512,
        _ => return false,
    };
    !key.is_empty() && UnparsedPublicKey::new(scheme, key).verify(message, signature).is_ok()
}

/// The RSA signature algorithm of a digest algorithm.
fn signature_of(digest: &[u8]) -> Option<&'static [u8]> {
    match digest {
        SHA256 => Some(RSA_SHA256),
        SHA384 => Some(RSA_SHA384),
        SHA512 => Some(RSA_SHA512),
        _ => None,
    }
}

fn hash(algorithm: &[u8], data: &[u8]) -> Option<Vec<u8>> {
    use sha2::Digest;
    match algorithm {
        SHA256 => Some(sha2::Sha256::digest(data).to_vec()),
        SHA384 => Some(sha2::Sha384::digest(data).to_vec()),
        SHA512 => Some(sha2::Sha512::digest(data).to_vec()),
        _ => None,
    }
}

/// A BIT STRING's bits, whole bytes only.
fn bits(content: &[u8]) -> Option<&[u8]> {
    content.split_first().filter(|(unused, _)| **unused == 0).map(|(_, bits)| bits)
}

/// A UTCTime (`YYMMDDHHMMSSZ`, 1950 to 2049) or GeneralizedTime (`YYYYMMDDHHMMSS[.f]Z`). A fraction of
/// a second counts as the whole second after: never earlier than it was.
fn time_of(tag: u8, text: &[u8]) -> Option<SystemTime> {
    let text = std::str::from_utf8(text).ok()?.strip_suffix('Z')?;
    let (whole, fraction) = match text.split_once('.') {
        Some((whole, fraction)) if !fraction.is_empty() && fraction.bytes().all(|b| b.is_ascii_digit()) => {
            (whole, fraction.bytes().any(|b| b != b'0'))
        }
        Some(_) => return None,
        None => (text, false),
    };
    let (year, rest) = match tag {
        UTC_TIME if whole.len() == 12 => {
            let year: i32 = whole[..2].parse().ok()?;
            (if year < 50 { 2000 + year } else { 1900 + year }, &whole[2..])
        }
        GENERALIZED_TIME if whole.len() == 14 => (whole[..4].parse().ok()?, &whole[4..]),
        _ => return None,
    };
    if !whole.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let part = |at: usize| rest[at..at + 2].parse::<u8>().ok();
    let at = DateTime { year, month: part(0)?, day: part(2)?, hour: part(4)?, minute: part(6)? };
    let second = part(8)?;
    if DateTime::parse(&at.text()).ok()? != at || second > 59 {
        return None;
    }
    let seconds = at.seconds_as_utc() + i64::from(second) + i64::from(fraction);
    Some(UNIX_EPOCH + Duration::from_secs(u64::try_from(seconds).ok()?))
}

/// A TimeStampReq for `digest` (SHA-256), asking for the signer's certificates with the token.
#[cfg_attr(not(test), allow(dead_code))] // the sync server's
pub fn request(digest: &[u8; 32]) -> Vec<u8> {
    let algorithm = der(SEQUENCE, &[der(OID, SHA256), der(NULL, &[])].concat());
    let imprint = der(SEQUENCE, &[algorithm, der(OCTET_STRING, digest)].concat());
    der(SEQUENCE, &[der(INTEGER, &[1]), imprint, der(BOOLEAN, &[0xff])].concat())
}

/// The token of a TimeStampResp, if it was granted.
#[cfg_attr(not(test), allow(dead_code))] // the sync server's
pub fn token_of(response: &[u8]) -> Result<Vec<u8>, String> {
    let mut answer = Reader::new(response).sequence().ok_or("Not an answer of a timestamp authority")?;
    let status = answer.sequence().and_then(|mut s| s.next_of(INTEGER)).ok_or("Not an answer of a timestamp authority")?;
    // 0: granted, 1: granted with changes.
    if status != [0] && status != [1] {
        return Err(format!("The timestamp authority refused (status {status:?})"));
    }
    answer.raw_of(SEQUENCE).map(<[u8]>::to_vec).ok_or_else(|| "The timestamp authority sent no token".into())
}

/// Asks the authority at `url` to timestamp `digest`: its token, checked to be one.
#[cfg_attr(not(test), allow(dead_code))] // the sync server's
pub fn fetch(url: &str, digest: &[u8; 32]) -> Result<Vec<u8>, String> {
    let response = ureq::post(url)
        .timeout(TIMEOUT)
        .set("Content-Type", "application/timestamp-query")
        .send_bytes(&request(digest))
        .map_err(|e| format!("The timestamp authority cannot be reached: {e}"))?;
    let mut body = Vec::new();
    std::io::Read::read_to_end(&mut std::io::Read::take(response.into_reader(), MAX_RESPONSE), &mut body)
        .map_err(|e| format!("The timestamp authority's answer cannot be read: {e}"))?;
    let token = token_of(&body)?;
    verify(&token, digest)?;
    Ok(token)
}

/// A DER element.
#[cfg_attr(not(test), allow(dead_code))] // the sync server's
fn der(tag: u8, content: &[u8]) -> Vec<u8> {
    let length = content.len();
    let mut out = vec![tag];
    if length < 0x80 {
        out.push(length as u8);
    } else {
        let bytes: Vec<u8> = length.to_be_bytes().into_iter().skip_while(|b| *b == 0).collect();
        out.push(0x80 | bytes.len() as u8);
        out.extend(bytes);
    }
    out.extend_from_slice(content);
    out
}

/// Reads DER elements one after the other. Only what Bullseye reads: tags of one byte, definite
/// lengths of up to four bytes. Anything else ends it (`None`).
struct Reader<'a> {
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Reader<'a> {
        Reader { rest: bytes }
    }

    fn done(&self) -> bool {
        self.rest.is_empty()
    }

    fn peek(&self) -> Option<u8> {
        self.rest.first().copied()
    }

    /// The next element: its tag, its content and all of it.
    fn element(&mut self) -> Option<(u8, &'a [u8], &'a [u8])> {
        let (&tag, after) = self.rest.split_first()?;
        if tag & 0x1f == 0x1f {
            return None;
        }
        let (&first, after) = after.split_first()?;
        let (length, after) = if first < 0x80 {
            (usize::from(first), after)
        } else {
            let count = usize::from(first & 0x7f);
            if count == 0 || count > 4 || after.len() < count {
                return None;
            }
            let length = after[..count].iter().fold(0usize, |n, b| (n << 8) | usize::from(*b));
            (length, &after[count..])
        };
        if after.len() < length {
            return None;
        }
        let header = self.rest.len() - after.len();
        let whole = &self.rest[..header + length];
        self.rest = &after[length..];
        Some((tag, &after[..length], whole))
    }

    /// The content of the next element, if it has the tag.
    fn next_of(&mut self, tag: u8) -> Option<&'a [u8]> {
        if self.peek()? != tag {
            return None;
        }
        self.element().map(|(_, content, _)| content)
    }

    /// The whole of the next element, if it has the tag.
    fn raw_of(&mut self, tag: u8) -> Option<&'a [u8]> {
        if self.peek()? != tag {
            return None;
        }
        self.element().map(|(_, _, whole)| whole)
    }

    fn sequence(&mut self) -> Option<Reader<'a>> {
        self.next_of(SEQUENCE).map(Reader::new)
    }

    fn oid(&mut self) -> Option<&'a [u8]> {
        self.next_of(OID)
    }

    /// `[n] EXPLICIT`: what is inside.
    fn explicit(&mut self, n: u8) -> Option<Reader<'a>> {
        self.next_of(tagged(n)).map(Reader::new)
    }

    /// An optional element with this tag: what is inside, if it is there.
    fn optional(&mut self, tag: u8) -> Option<Reader<'a>> {
        self.next_of(tag).map(Reader::new)
    }
}

/// Base64 (standard, padded), for a token in text.
#[cfg_attr(not(test), allow(dead_code))] // the sync server's
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            out.push(if i <= chunk.len() { char::from(ALPHABET[(n >> (18 - 6 * i) & 63) as usize]) } else { '=' });
        }
    }
    out
}

/// What `base64` made, or `None` for anything else.
pub fn unbase64(text: &str) -> Option<Vec<u8>> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for (index, chunk) in bytes.chunks(4).enumerate() {
        let last = index == bytes.len() / 4 - 1;
        let padding = chunk.iter().rev().take_while(|c| **c == b'=').count();
        if padding > 2 || (padding > 0 && !last) {
            return None;
        }
        let mut n = 0u32;
        for (i, c) in chunk[..4 - padding].iter().enumerate() {
            n |= u32::from(value(*c)?) << (18 - 6 * i);
        }
        let take = 3 - padding;
        if n & ((1 << (8 * padding)) - 1) != 0 {
            return None;
        }
        out.extend_from_slice(&n.to_be_bytes()[1..1 + take]);
    }
    Some(out)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    const DIGICERT: &[u8] = include_bytes!("fixtures/digicert.tsr");
    const TEST: &[u8] = include_bytes!("fixtures/test.tsr");
    pub const TEST_ROOT: &[u8] = include_bytes!("fixtures/test-root.der");
    const SIGNER_KEY: &[u8] = include_bytes!("fixtures/test-signer.pk8");
    const SIGNER: &[u8] = include_bytes!("fixtures/for-timestamps.der");
    const ISSUER: &[u8] = include_bytes!("fixtures/test-issuer.der");

    /// The digest every fixture timestamps: of "bullseye fixture".
    fn fixture() -> [u8; 32] {
        digest(b"bullseye fixture")
    }

    fn token(response: &[u8]) -> Vec<u8> {
        token_of(response).unwrap()
    }

    fn test_roots() -> Vec<&'static [u8]> {
        vec![TEST_ROOT]
    }

    /// A token of the test authority for `digest` at `at`, made here: as an authority makes one. The
    /// tests of syncing use these (`verify` pins the test authority in tests).
    pub fn make(digest: &[u8; 32], at: SystemTime) -> Vec<u8> {
        use ring::signature::{RsaKeyPair, RSA_PKCS1_SHA256};
        let unix = at.duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
        let day = DateTime { year: 1970, month: 1, day: 1, hour: 0, minute: 0 };
        let mut date = day;
        // The date of `unix`: days from 1970, a year and a month at a time.
        let mut days = unix.div_euclid(86_400);
        while days >= if crate::datetime::days_in_month(date.year, 2) == 29 { 366 } else { 365 } {
            days -= if crate::datetime::days_in_month(date.year, 2) == 29 { 366 } else { 365 };
            date.year += 1;
        }
        while days >= i64::from(crate::datetime::days_in_month(date.year, date.month)) {
            days -= i64::from(crate::datetime::days_in_month(date.year, date.month));
            date.month += 1;
        }
        date.day = 1 + days as u8;
        let of_day = unix.rem_euclid(86_400);
        let time =
            format!("{:04}{:02}{:02}{:02}{:02}{:02}Z", date.year, date.month, date.day, of_day / 3600, of_day / 60 % 60, of_day % 60);
        let sha256 = der(SEQUENCE, &[der(OID, SHA256), der(NULL, &[])].concat());
        let imprint = der(SEQUENCE, &[sha256.clone(), der(OCTET_STRING, digest)].concat());
        let tst = der(
            SEQUENCE,
            &[der(INTEGER, &[1]), der(OID, &[0x2a, 0x03, 0x04]), imprint, der(INTEGER, &[7]), der(GENERALIZED_TIME, time.as_bytes())]
                .concat(),
        );
        let signer = Certificate::read(SIGNER).unwrap();
        let attribute = |kind: &[u8], value: Vec<u8>| der(SEQUENCE, &[der(OID, kind), der(SET, &value)].concat());
        let attributes =
            [attribute(CONTENT_TYPE, der(OID, TST_INFO)), attribute(MESSAGE_DIGEST, der(OCTET_STRING, &hash(SHA256, &tst).unwrap()))]
                .concat();
        let key = RsaKeyPair::from_pkcs8(SIGNER_KEY).unwrap();
        let mut signature = vec![0; key.public().modulus_len()];
        key.sign(&RSA_PKCS1_SHA256, &ring::rand::SystemRandom::new(), &der(SET, &attributes), &mut signature).unwrap();
        let id = der(SEQUENCE, &[signer.issuer.to_vec(), der(INTEGER, signer.serial)].concat());
        let signer_info = der(
            SEQUENCE,
            &[
                der(INTEGER, &[1]),
                id,
                sha256.clone(),
                der(tagged(0), &attributes),
                der(SEQUENCE, &[der(OID, RSA), der(NULL, &[])].concat()),
                der(OCTET_STRING, &signature),
            ]
            .concat(),
        );
        let content = der(SEQUENCE, &[der(OID, TST_INFO), der(tagged(0), &der(OCTET_STRING, &tst))].concat());
        let signed = der(
            SEQUENCE,
            &[der(INTEGER, &[3]), der(SET, &sha256), content, der(tagged(0), &[SIGNER, ISSUER].concat()), der(SET, &signer_info)].concat(),
        );
        der(SEQUENCE, &[der(OID, SIGNED_DATA), der(tagged(0), &signed)].concat())
    }

    #[test]
    fn digicerts_own_token_is_believed_with_its_time() {
        let at = verify(&token(DIGICERT), &fixture()).unwrap();
        // 2026-11-02 09:56:50 UTC, when it was fetched.
        assert_eq!(at, UNIX_EPOCH + Duration::from_secs(1_793_613_410));
        assert!(verify(&token(DIGICERT), &digest(b"something else")).unwrap_err().contains("something else"));
    }

    #[test]
    fn the_test_authority_is_believed_only_where_it_is_pinned() {
        let test = token(TEST);
        assert!(verify_with(&test, &fixture(), &test_roots()).is_ok());
        for (name, response) in
            [("sha384", &include_bytes!("fixtures/test-sha384.tsr")[..]), ("sha512", &include_bytes!("fixtures/test-sha512.tsr")[..])]
        {
            assert!(verify_with(&token(response), &fixture(), &test_roots()).is_ok(), "{name}");
        }
        assert!(verify_with(&test, &fixture(), ROOTS).unwrap_err().contains("known authority"));
        // Not DigiCert's either, with only the test root.
        assert!(verify_with(&token(DIGICERT), &fixture(), &test_roots()).unwrap_err().contains("known authority"));
    }

    #[test]
    fn a_token_whose_chain_fails_is_none() {
        let check = |response: &[u8]| verify_with(&token(response), &fixture(), &test_roots()).unwrap_err();
        for (response, why) in [
            (&include_bytes!("fixtures/test-expired.tsr")[..], "not valid then"),
            (include_bytes!("fixtures/test-unknown-root.tsr"), "known authority"),
            (include_bytes!("fixtures/test-no-chain.tsr"), "known authority"),
        ] {
            let error = check(response);
            assert!(error.contains(why), "{error}");
        }
    }

    #[test]
    fn only_a_certificate_marked_for_timestamping_alone_signs_one() {
        assert!(Certificate::read(SIGNER).unwrap().stamps());
        assert!(!Certificate::read(include_bytes!("fixtures/not-for-timestamps.der")).unwrap().stamps());
        assert!(!Certificate::read(ISSUER).unwrap().stamps());
    }

    /// Any byte of a token changed: it is no timestamp, or the same one (a field nobody signs, such as
    /// a version); never another time, and never a panic.
    #[test]
    fn a_token_changed_anywhere_gives_no_other_time() {
        for original in [token(DIGICERT), token(TEST)] {
            let roots = [ROOTS, &test_roots()].concat();
            let at = verify_with(&original, &fixture(), &roots).unwrap();
            for index in 0..original.len() {
                for flip in [0x01, 0x80, 0xff] {
                    let mut changed = original.clone();
                    changed[index] ^= flip;
                    if let Ok(other) = verify_with(&changed, &fixture(), &roots) {
                        assert_eq!(other, at, "byte {index}");
                    }
                }
            }
            for length in 0..original.len() {
                assert!(verify_with(&original[..length], &fixture(), &roots).is_err());
            }
        }
    }

    #[test]
    fn a_made_token_is_believed_at_its_time() {
        let at = UNIX_EPOCH + Duration::from_secs(1_894_060_800 + 10 * 3600 + 61);
        let made = make(&digest(b"x"), at);
        assert_eq!(verify(&made, &digest(b"x")), Ok(at));
        assert!(verify_with(&made, &digest(b"x"), ROOTS).is_err());
        assert!(verify(&made, &digest(b"y")).is_err());
        // A leap year's last day, and the turn of a year.
        for text in ["2028-12-31 23:59", "2029-01-01 00:00", "2028-02-29 12:00"] {
            let at = UNIX_EPOCH + Duration::from_secs(DateTime::parse(text).unwrap().seconds_as_utc() as u64);
            assert_eq!(verify(&make(&digest(b"x"), at), &digest(b"x")), Ok(at), "{text}");
        }
    }

    #[test]
    fn times_are_read_never_earlier_than_they_are() {
        let at = |seconds: u64| Some(UNIX_EPOCH + Duration::from_secs(seconds));
        assert_eq!(time_of(GENERALIZED_TIME, b"20261102095620Z"), at(1_793_613_380));
        assert_eq!(time_of(GENERALIZED_TIME, b"20261102095620.25Z"), at(1_793_613_381));
        assert_eq!(time_of(GENERALIZED_TIME, b"20261102095620.000Z"), at(1_793_613_380));
        assert_eq!(time_of(UTC_TIME, b"261102095620Z"), at(1_793_613_380));
        assert_eq!(time_of(UTC_TIME, b"700101000000Z"), at(0));
        for (tag, wrong) in [
            (GENERALIZED_TIME, &b"20261102095620"[..]),
            (GENERALIZED_TIME, b"20261102095620.Z"),
            (GENERALIZED_TIME, b"20261302095620Z"),
            (GENERALIZED_TIME, b"20260230095620Z"),
            (GENERALIZED_TIME, b"20261102095660Z"),
            (GENERALIZED_TIME, b"2026110209562Z"),
            (GENERALIZED_TIME, b"2026110209562xZ"),
            (UTC_TIME, b"20261102095620Z"),
            (OCTET_STRING, b"261102095620Z"),
            (GENERALIZED_TIME, b"19691231235959Z"),
            (GENERALIZED_TIME, &[0xff]),
        ] {
            assert_eq!(time_of(tag, wrong), None, "{wrong:?}");
        }
    }

    #[test]
    fn a_request_is_what_an_authority_reads() {
        // TimeStampReq: version 1, the SHA-256 (with its NULL parameters) of what is asked for, and the
        // signer's certificates asked for.
        let mut expected = vec![0x30, 0x39, 0x02, 0x01, 0x01, 0x30, 0x31, 0x30, 0x0d, 0x06, 0x09];
        expected.extend_from_slice(SHA256);
        expected.extend_from_slice(&[0x05, 0x00, 0x04, 0x20]);
        expected.extend_from_slice(&fixture());
        expected.extend_from_slice(&[0x01, 0x01, 0xff]);
        assert_eq!(request(&fixture()), expected);
        assert_eq!(der(OCTET_STRING, &[7; 300])[..4], [OCTET_STRING, 0x82, 0x01, 0x2c]);
    }

    #[test]
    fn an_answer_gives_its_token_only_when_granted() {
        assert_eq!(token_of(DIGICERT).unwrap()[..2], [0x30, 0x82]);
        let refused = der(SEQUENCE, &der(SEQUENCE, &der(INTEGER, &[2])));
        assert!(token_of(&refused).unwrap_err().contains("refused"));
        let empty = der(SEQUENCE, &der(SEQUENCE, &der(INTEGER, &[0])));
        assert!(token_of(&empty).unwrap_err().contains("no token"));
        assert!(token_of(b"<html>").is_err());
    }

    #[test]
    fn the_reader_refuses_what_it_does_not_read() {
        // A tag of more than one byte, a length of none, five or more bytes than there are.
        for bytes in [&[0x1f, 0x01, 0x00][..], &[0x30, 0x80], &[0x30, 0x85, 1, 1, 1, 1, 1], &[0x30, 0x82, 0x01], &[0x30, 0x05, 1]] {
            assert!(Reader::new(bytes).element().is_none(), "{bytes:?}");
        }
        assert_eq!(bits(&[1, 0xff]), None);
    }

    #[test]
    fn base64_goes_there_and_back() {
        for bytes in [&b""[..], b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar", &[0, 255, 128, 7]] {
            assert_eq!(unbase64(&base64(bytes)).as_deref(), Some(bytes));
        }
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(b"fo"), "Zm8=");
        for wrong in ["Zm9", "Zm9v!mFy", "Z===", "Zm8=Zm8=", "Zm9=", "Zm8x="] {
            assert_eq!(unbase64(wrong), None, "{wrong}");
        }
    }

    /// An authority on this computer, answering once with `answer`: what the server asks it.
    #[test]
    fn the_server_asks_an_authority_and_checks_its_token() {
        use std::io::{Read, Write};
        let serve = |answer: Vec<u8>, status: &'static str| {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/", listener.local_addr().unwrap());
            let thread = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 4096];
                // Until the body (the request, 59 bytes) is in.
                while !request.ends_with(&super::request(&fixture())) {
                    let read = stream.read(&mut buffer).unwrap();
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..read]);
                }
                let header = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", answer.len());
                // The client may hang up once it has read enough.
                let _ = stream.write_all(header.as_bytes()).and_then(|()| stream.write_all(&answer));
                request
            });
            (url, thread)
        };
        let (url, thread) = serve(DIGICERT.to_vec(), "200 OK");
        assert_eq!(fetch(&url, &fixture()), Ok(token(DIGICERT)));
        let asked = thread.join().unwrap();
        assert!(String::from_utf8_lossy(&asked).contains("application/timestamp-query"));
        assert!(asked.ends_with(&request(&fixture())));
        // A token for something else, a refusal and an error are not taken.
        let (url, thread) = serve(DIGICERT.to_vec(), "200 OK");
        assert!(fetch(&url, &digest(b"else")).is_err());
        thread.join().unwrap();
        let (url, thread) = serve(b"no".to_vec(), "500 Oops");
        assert!(fetch(&url, &fixture()).unwrap_err().contains("cannot be reached"));
        thread.join().unwrap();
        assert!(fetch("http://127.0.0.1:1/", &fixture()).is_err());
    }
}
