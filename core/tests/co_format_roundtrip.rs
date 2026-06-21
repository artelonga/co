//! CO-86 — `.co` round-trip integration tests over a corpus of real-world
//! content shapes (modeled on quilomboaraucaria + ArteLonga markdown).
//!
//! Each fixture is encoded → bytes → decoded and asserted to reproduce the
//! original body byte-for-byte with frontmatter equivalence (order-independent,
//! whitespace-tolerant). The corpus exercises the shapes that actually occur in
//! the platform: Obsidian-style gardens with empty/null fields, Portuguese
//! frontmatter keys, task/event types, nested values and bare markdown.

use co::co_format::{self, codec, encryption, signature};

/// Frontmatter shapes harvested from the live universes. Empty values
/// (`permalink:`, `tags:`), Portuguese keys (`titulo`, `responsavel`), booleans
/// (`draft`/`publicado`) and dates are all represented.
fn corpus() -> Vec<(&'static str, String)> {
    vec![
        (
            "garden-empty-fields",
            "---\ntitle: Gestão de Carreira\ndescription: Conheça o serviço de gestão de carreira.\npermalink: \ndate: 2024-11-28\ntime: 06:47\ntype: garden\ntags: \ndraft: false\n---\n\n# Gestão de Carreira\n\nUm parágrafo com conteúdo.\n".to_string(),
        ),
        (
            "node-with-subtype",
            "---\ntitle: ArteLonga\ndescription: Arte Longa é uma agência.\npermalink: \ndate: 2024-11-01\ntime: 11:04\ntype: node\nsubtype: \ntags: \ndraft: false\n---\n\nCorpo do node.\n".to_string(),
        ),
        (
            "doc-minimal",
            "---\ntype: doc\ntitle: CLAUDE.md\n---\n\n# Conteúdo\n".to_string(),
        ),
        (
            "missao-portuguese-keys",
            "---\ntype: missao\ntitulo: Missão Exemplo\nslug: missao-exemplo\nfuncao: \nresponsavel: \ndescricao: \ntags: []\npublicado: true\ndata: \nstatus: ativa\nprioridade: \n---\n\nDescrição da missão.\n".to_string(),
        ),
        (
            "task-typed-fields",
            "---\ntitle: Implementar formato .co\ntype: task\nstatus: doing\npriority: high\nparent: CO-20\nassignee: yuri\ntags:\n- format\n- protobuf\n---\n\nGIVEN ... WHEN ... THEN ...\n".to_string(),
        ),
        (
            "evento-with-numbers",
            "---\ntitle: Evento Astronômico\ntype: evento\norder: 3\nratio: 1.5\nconfirmado: true\ntags:\n- astro\n---\n\n## Detalhes\n\nCorpo do evento.\n".to_string(),
        ),
        (
            "bare-markdown-no-frontmatter",
            "# Sem frontmatter\n\nApenas corpo de texto, com acento e emoji 🌱.\n".to_string(),
        ),
    ]
}

/// Body byte-identical + frontmatter equivalence across the whole corpus.
#[test]
fn corpus_roundtrips_body_and_frontmatter() {
    for (name, md) in corpus() {
        let co = co_format::from_markdown(&md);
        let bytes = co_format::to_bytes(&co);
        let decoded = co_format::from_bytes(&bytes).expect("decode");
        let out = co_format::to_markdown(&decoded).expect("to_markdown");

        // Body must be byte-identical.
        let orig_body = split_body(&md);
        let out_body = split_body(&out);
        assert_eq!(orig_body, out_body, "body mismatch for fixture `{name}`");

        // Frontmatter must be equivalent (order-independent).
        assert_eq!(
            co_format::frontmatter_value(&md),
            co_format::frontmatter_value(&out),
            "frontmatter mismatch for fixture `{name}`",
        );
    }
}

/// A ~10 KB markdown file round-trips with byte-identical body and frontmatter
/// equivalence (acceptance criterion #1).
#[test]
fn ten_kb_file_roundtrips() {
    let mut md = String::from(
        "---\ntitle: Documento Grande\ntype: page\nstatus: done\ntags:\n- grande\n- teste\n---\n\n",
    );
    while md.len() < 10_240 {
        md.push_str("## Seção\n\nParágrafo com conteúdo representativo e acentuação. ");
        md.push_str("Repetição estrutural para encher o documento até ~10 KB.\n\n");
    }
    assert!(md.len() >= 10_240);

    let co = co_format::from_markdown(&md);
    let bytes = co_format::to_bytes(&co);
    let decoded = co_format::from_bytes(&bytes).unwrap();
    let out = co_format::to_markdown(&decoded).unwrap();

    assert_eq!(split_body(&md), split_body(&out));
    assert_eq!(
        co_format::frontmatter_value(&md),
        co_format::frontmatter_value(&out)
    );
}

/// Compression layer round-trips and shrinks repetitive content.
#[test]
fn compressed_body_roundtrips() {
    let md = format!(
        "---\ntitle: T\n---\n{}",
        "linha repetida para comprimir bem\n".repeat(300)
    );
    let mut co = co_format::from_markdown(&md);
    let plain = co_format::canonical_body(&co).unwrap();

    codec::compress_body(&mut co, codec::DEFAULT_LEVEL).unwrap();
    let bytes = co_format::to_bytes(&co);
    let decoded = co_format::from_bytes(&bytes).unwrap();
    assert_eq!(co_format::canonical_body(&decoded).unwrap(), plain);
}

/// Acceptance criterion #2 — an encrypted body decrypts only with the right
/// key; tampering changes the SHA-256 / fails AEAD verification.
#[test]
fn encrypted_body_decrypts_only_with_right_key() {
    let md = "---\ntitle: Secreto\n---\nconteúdo confidencial\n";
    let mut co = co_format::from_markdown(md);
    let plaintext = co_format::canonical_body(&co).unwrap();

    let key = encryption::generate_key();
    encryption::encrypt_body(&mut co, &key).unwrap();
    assert!(encryption::is_encrypted(&co));

    // Round-trip through bytes, then decrypt with the right key.
    let bytes = co_format::to_bytes(&co);
    let mut decoded = co_format::from_bytes(&bytes).unwrap();
    let mut good = decoded.clone();
    encryption::decrypt_body(&mut good, &key).unwrap();
    assert_eq!(co_format::canonical_body(&good).unwrap(), plaintext);

    // Wrong key fails.
    let wrong = encryption::generate_key();
    assert!(encryption::decrypt_body(&mut decoded.clone(), &wrong).is_err());

    // Tampering the ciphertext fails AEAD verification.
    if let Some(co::co_format::co_file::Body::Encrypted(enc)) = decoded.body.as_mut() {
        enc.ciphertext[0] ^= 0xff;
    }
    assert!(encryption::decrypt_body(&mut decoded, &key).is_err());
}

/// Acceptance criterion #3 — a signed `.co` verifies under the signer's pubkey;
/// mutating any byte of the body fails verification.
#[test]
fn signed_body_verifies_and_detects_mutation() {
    let md = "---\ntitle: Assinado\n---\ncorpo assinado\n";
    let mut co = co_format::from_markdown(md);
    let (sk, vk) = signature::generate_keypair();
    signature::sign(&mut co, &sk).unwrap();

    // Survives a bytes round-trip and verifies.
    let bytes = co_format::to_bytes(&co);
    let decoded = co_format::from_bytes(&bytes).unwrap();
    assert!(signature::verify(&decoded, &vk).unwrap());

    // Mutating the body without re-signing fails.
    let mut tampered = decoded.clone();
    tampered.body = Some(co::co_format::co_file::Body::Markdown(
        b"corpo adulterado\n".to_vec(),
    ));
    assert!(!signature::verify(&tampered, &vk).unwrap());

    // A different pubkey fails too.
    let (_, other_vk) = signature::generate_keypair();
    assert!(!signature::verify(&decoded, &other_vk).unwrap());
}

/// Acceptance criterion #6 — `inspect` reports the documented fields.
#[test]
fn inspect_reports_all_fields() {
    let md = "---\ntitle: T\n---\ncorpo\n";

    // Plain.
    let co = co_format::from_markdown(md);
    let bytes = co_format::to_bytes(&co);
    let info = co_format::inspect(&co, bytes.len());
    assert_eq!(info.version, co_format::SCHEMA_VERSION);
    assert_eq!(info.content_type, "co/markdown");
    assert!(info.size_uncompressed > 0);
    assert_eq!(info.size_on_wire, bytes.len() as i64);
    assert!(!info.signed);
    assert!(!info.encrypted);
    assert_eq!(info.attachment_count, 0);

    // Signed + encrypted flags flip.
    let mut secured = co_format::from_markdown(md);
    let (sk, _) = signature::generate_keypair();
    signature::sign(&mut secured, &sk).unwrap();
    let key = encryption::generate_key();
    encryption::encrypt_body(&mut secured, &key).unwrap();
    let info2 = co_format::inspect(&secured, co_format::to_bytes(&secured).len());
    assert!(info2.signed);
    assert!(info2.encrypted);
    assert_eq!(info2.content_type, "co/encrypted");
}

/// The survey-derived fixture corpus on disk (`tests/fixtures/co_format/`),
/// modeled on real quilomboaraucaria + ArteLonga content. These exercise
/// shapes the inline corpus does not — notably `referencias.md`, whose
/// frontmatter carries a list of heterogeneous mappings with integer fields
/// (the hardest case for the `extra` Struct round-trip).
#[test]
fn fixture_files_roundtrip() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/co_format");
    let mut count = 0;
    for entry in std::fs::read_dir(&dir).expect("read fixtures dir") {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let md = std::fs::read_to_string(&path).unwrap();

        let co = co_format::from_markdown(&md);
        let bytes = co_format::to_bytes(&co);
        let decoded = co_format::from_bytes(&bytes).expect("decode");
        let out = co_format::to_markdown(&decoded).expect("to_markdown");

        assert_eq!(split_body(&md), split_body(&out), "body mismatch in {name}");
        assert_eq!(
            co_format::frontmatter_value(&md),
            co_format::frontmatter_value(&out),
            "frontmatter mismatch in {name}",
        );
        co_format::validate(&decoded).unwrap_or_else(|e| panic!("validate {name}: {e}"));
        count += 1;
    }
    assert!(
        count >= 7,
        "expected the full fixture corpus, found {count}"
    );
}

/// Split a markdown string at its frontmatter and return the body verbatim.
fn split_body(md: &str) -> &str {
    if let Some(rest) = md.strip_prefix("---\n")
        && let Some(idx) = rest.find("\n---\n")
    {
        return &rest[idx + 5..];
    }
    md
}
