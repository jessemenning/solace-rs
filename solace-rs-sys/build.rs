extern crate bindgen;
use sha2::{Digest, Sha256};
use std::io::{BufWriter, Read, Write};
use std::sync::Arc;
use std::{env, path::Path, path::PathBuf};
use ureq::Agent;

// Solace C API version — keep in sync with the platform constants below.
// When bumping this version, recompute all SOLCLIENT_EXPECTED_SHA256 constants.
const SOLCLIENT_VERSION: &str = "7.33.2.3";
const SOLCLIENT_FOLDER_NAME: &str = "solclient-7.33.2.3";

// Sentinel file written after a successful extraction.  Its presence proves the
// library tree is complete; without it the extraction is re-attempted.
const EXTRACTION_MARKER: &str = ".solclient-extracted";

// ── Archive filenames ─────────────────────────────────────────────────────────

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const SOLCLIENT_ARCHIVE_PATH: &str = "solclient_Win_vs2015_7.33.2.3.tar.gz";

#[cfg(target_os = "macos")]
const SOLCLIENT_ARCHIVE_PATH: &str = "solclient_Darwin-universal2_opt_7.33.2.3.tar.gz";

#[cfg(all(target_os = "linux", target_arch = "x86_64", not(target_env = "musl")))]
const SOLCLIENT_ARCHIVE_PATH: &str = "solclient_Linux26-x86_64_opt_7.33.2.3.tar.gz";

#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const SOLCLIENT_ARCHIVE_PATH: &str = "solclient_Linux-aarch64_opt_7.33.2.3.tar.gz";

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "musl"))]
const SOLCLIENT_ARCHIVE_PATH: &str = "solclient_Linux_musl-x86_64_opt_7.33.2.3.tar.gz";

// Unsupported platform (e.g. Windows aarch64) — empty fallback so the code compiles when
// SOLCLIENT_LIB_PATH bypasses the download path at runtime.
#[cfg(not(any(
    all(target_os = "windows", target_arch = "x86_64"),
    target_os = "macos",
    all(target_os = "linux", target_arch = "x86_64"),
    all(target_os = "linux", target_arch = "aarch64"),
)))]
const SOLCLIENT_ARCHIVE_PATH: &str = "";

// ── Official download URLs ────────────────────────────────────────────────────

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const SOLCLIENT_OFFICIAL_URL: &str = "https://products.solace.com/download/C_API_VS2015";

#[cfg(target_os = "macos")]
const SOLCLIENT_OFFICIAL_URL: &str = "https://products.solace.com/download/C_API_OSX";

#[cfg(all(target_os = "linux", target_arch = "x86_64", not(target_env = "musl")))]
const SOLCLIENT_OFFICIAL_URL: &str = "https://products.solace.com/download/C_API_LINUX64";

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "musl"))]
const SOLCLIENT_OFFICIAL_URL: &str = "https://products.solace.com/download/C_API_MUSL";

// No known official URL for this platform (Linux aarch64, Windows non-x86_64, etc.).
// Users must supply SOLCLIENT_TARBALL_URL or SOLCLIENT_LIB_PATH.
#[cfg(not(any(
    all(target_os = "windows", target_arch = "x86_64"),
    target_os = "macos",
    all(target_os = "linux", target_arch = "x86_64"),
)))]
const SOLCLIENT_OFFICIAL_URL: &str = "";

// ── Expected SHA-256 checksums ────────────────────────────────────────────────
// Compute with: sha256sum <tarball>  (Linux / macOS)
//               certutil -hashfile <tarball> SHA256  (Windows)
// Update whenever SOLCLIENT_VERSION changes.
//
// Set SOLCLIENT_SKIP_CHECKSUM=1 to bypass verification (not recommended).

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const SOLCLIENT_EXPECTED_SHA256: &str =
    "TODO: sha256sum solclient_Win_vs2015_7.33.2.3.tar.gz";

#[cfg(target_os = "macos")]
const SOLCLIENT_EXPECTED_SHA256: &str =
    "TODO: sha256sum solclient_Darwin-universal2_opt_7.33.2.3.tar.gz";

#[cfg(all(target_os = "linux", target_arch = "x86_64", not(target_env = "musl")))]
const SOLCLIENT_EXPECTED_SHA256: &str =
    "TODO: sha256sum solclient_Linux26-x86_64_opt_7.33.2.3.tar.gz";

#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const SOLCLIENT_EXPECTED_SHA256: &str =
    "TODO: sha256sum solclient_Linux-aarch64_opt_7.33.2.3.tar.gz";

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "musl"))]
const SOLCLIENT_EXPECTED_SHA256: &str =
    "TODO: sha256sum solclient_Linux_musl-x86_64_opt_7.33.2.3.tar.gz";

// Unsupported platform — checksum verification skipped.
#[cfg(not(any(
    all(target_os = "windows", target_arch = "x86_64"),
    target_os = "macos",
    all(target_os = "linux", target_arch = "x86_64"),
    all(target_os = "linux", target_arch = "aarch64"),
)))]
const SOLCLIENT_EXPECTED_SHA256: &str = "";

// ─────────────────────────────────────────────────────────────────────────────

fn build_ureq_agent() -> Agent {
    // install_default() returns Err if a provider is already registered — ignore
    // that error so this function is safe to call more than once (e.g. in retry loops).
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut root_store = rustls::RootCertStore::empty();
    for cert in rustls_native_certs::load_native_certs().expect("could not load platform certs") {
        // Skip malformed individual certificates rather than aborting the build on
        // machines with unusual system cert stores.
        let _ = root_store.add(cert);
    }
    let tls_config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    ureq::builder().tls_config(Arc::new(tls_config)).build()
}

/// Downloads the tarball at `url` to `tarball_path`, verifying its SHA-256 against
/// `expected_sha256` (unless it starts with "TODO" or `SOLCLIENT_SKIP_CHECKSUM` is set),
/// then extracts it into `tarball_unpack_path` and writes a completion marker.
fn download_and_unpack(
    url: &str,
    tarball_path: &Path,
    tarball_unpack_path: &Path,
    expected_sha256: &str,
) {
    // Validate the URL scheme before touching the network.
    assert!(
        url.starts_with("https://"),
        "Download URL must use https://, got: {url}"
    );

    eprintln!("Downloading Solace C API {SOLCLIENT_VERSION} from {url}");

    // Stream directly to disk, computing SHA-256 in one pass — avoids buffering
    // the full tarball (~30–60 MB) in RAM.
    let response = build_ureq_agent()
        .get(url)
        .call()
        .unwrap_or_else(|e| panic!("Download failed for {url}: {e}"));

    let file = std::fs::File::create(tarball_path)
        .unwrap_or_else(|e| panic!("Could not create {}: {e}", tarball_path.display()));
    let mut writer = BufWriter::new(file);
    let mut hasher = Sha256::new();
    let mut reader = response.into_reader();
    let mut buf = [0u8; 65536];
    loop {
        let n = reader
            .read(&mut buf)
            .unwrap_or_else(|e| panic!("Read error while downloading {url}: {e}"));
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        writer
            .write_all(&buf[..n])
            .unwrap_or_else(|e| panic!("Write error to {}: {e}", tarball_path.display()));
    }
    writer
        .flush()
        .unwrap_or_else(|e| panic!("Flush error for {}: {e}", tarball_path.display()));

    // Verify integrity unless explicitly skipped or checksum not yet filled in.
    let skip_checksum = env::var("SOLCLIENT_SKIP_CHECKSUM")
        .ok()
        .filter(|s| !s.is_empty())
        .is_some();

    if !skip_checksum && !expected_sha256.is_empty() && !expected_sha256.starts_with("TODO") {
        let actual = format!("{:x}", hasher.finalize());
        if actual != expected_sha256 {
            // Remove the corrupt file so the next build starts fresh.
            let _ = std::fs::remove_file(tarball_path);
            panic!(
                "SHA-256 mismatch for {url}\n  \
                 expected: {expected_sha256}\n  \
                 actual:   {actual}\n\
                 The tarball has been removed. This may indicate a corrupted download \
                 or a supply-chain attack. Re-run the build to retry."
            );
        }
        eprintln!("SHA-256 verified OK.");
    } else if !skip_checksum {
        eprintln!(
            "WARNING: SOLCLIENT_EXPECTED_SHA256 is not set for this platform. \
             Integrity of the downloaded tarball has NOT been verified. \
             Fill in the SHA-256 constant in build.rs to enable verification."
        );
    }

    // Extract the tarball.
    let file_gz = std::fs::File::open(tarball_path)
        .unwrap_or_else(|e| panic!("Could not open {}: {e}", tarball_path.display()));
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file_gz));

    std::fs::create_dir_all(tarball_unpack_path).unwrap_or_else(|e| {
        panic!(
            "Could not create {}: {e}",
            tarball_unpack_path.display()
        )
    });

    for entry in archive
        .entries()
        .unwrap_or_else(|e| panic!("Failed to read archive entries: {e}"))
    {
        let mut entry = entry.unwrap_or_else(|e| panic!("Failed to read archive entry: {e}"));

        let entry_path = entry
            .path()
            .unwrap_or_else(|e| panic!("Failed to read entry path: {e}"))
            .into_owned();
        let strip_path = entry_path.iter().skip(1).collect::<PathBuf>();

        // Guard against path-traversal attacks in crafted tarballs.
        // Check the stripped path before joining — starts_with() alone is not
        // sufficient because it is lexical and does not resolve `..` components.
        assert!(
            strip_path.is_relative(),
            "Absolute path in archive entry: {}",
            entry_path.display()
        );
        assert!(
            !strip_path
                .components()
                .any(|c| c == std::path::Component::ParentDir),
            "Path traversal (.. component) in archive entry: {}",
            entry_path.display()
        );

        let dest = tarball_unpack_path.join(&strip_path);
        entry
            .unpack(&dest)
            .unwrap_or_else(|e| panic!("Failed to unpack {}: {e}", dest.display()));
        println!("> {}", dest.display());
    }

    // Write the marker so future builds know extraction completed successfully.
    std::fs::File::create(tarball_unpack_path.join(EXTRACTION_MARKER))
        .unwrap_or_else(|e| panic!("Could not write extraction marker: {e}"));
}

fn main() {
    // Skip everything when building docs.
    if std::env::var("DOCS_RS").is_ok() {
        return;
    }

    // Tell Cargo to re-run this script only when relevant inputs change,
    // avoiding unnecessary full rebuilds on every incremental cargo invocation.
    println!("cargo:rerun-if-env-changed=SOLCLIENT_LIB_PATH");
    println!("cargo:rerun-if-env-changed=SOLCLIENT_TARBALL_URL");
    println!("cargo:rerun-if-env-changed=SOLCLIENT_SKIP_CHECKSUM");
    println!("cargo:rerun-if-env-changed=DOCS_RS");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let solclient_folder_path = out_dir.join(SOLCLIENT_FOLDER_NAME);

    let lib_dir = if let Ok(path) = env::var("SOLCLIENT_LIB_PATH") {
        PathBuf::from(path)
    } else {
        // Treat an empty SOLCLIENT_TARBALL_URL (e.g. an unset GitHub Actions secret
        // expanded to "") the same as the variable not being set at all.
        let solclient_tarball_url = env::var("SOLCLIENT_TARBALL_URL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                if SOLCLIENT_OFFICIAL_URL.is_empty() {
                    panic!(
                        "No official download URL is known for this platform.\n\
                         Set SOLCLIENT_TARBALL_URL to a URL pointing to the Solace C API \
                         {SOLCLIENT_FOLDER_NAME} tarball for your platform, or set \
                         SOLCLIENT_LIB_PATH to a directory containing the pre-extracted \
                         library files."
                    );
                }
                SOLCLIENT_OFFICIAL_URL.to_string()
            });

        let solclient_tarball_path = out_dir.join(SOLCLIENT_ARCHIVE_PATH);
        let extraction_marker = solclient_folder_path.join(EXTRACTION_MARKER);

        if !extraction_marker.exists() {
            // Directory may exist but be incomplete from a prior interrupted extraction.
            eprintln!("Solclient not found or extraction was incomplete. Downloading...");
            download_and_unpack(
                &solclient_tarball_url,
                &solclient_tarball_path,
                &solclient_folder_path,
                SOLCLIENT_EXPECTED_SHA256,
            );
        }

        solclient_folder_path.join("lib")
    };

    println!(
        "cargo:rustc-link-search=native={}",
        lib_dir.as_path().display()
    );

    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-lib=dylib=gssapi_krb5");
        // macOS: libsolclient.a does NOT embed OpenSSL (unlike Linux 7.33+).
        // Link against Homebrew OpenSSL 3 — arm64 path first, then x86_64 fallback.
        println!("cargo:rustc-link-search=native=/opt/homebrew/opt/openssl@3/lib");
        println!("cargo:rustc-link-search=native=/usr/local/opt/openssl@3/lib");
        println!("cargo:rustc-link-lib=dylib=ssl");
        println!("cargo:rustc-link-lib=dylib=crypto");
    }

    cfg_if::cfg_if! {
        if #[cfg(target_os = "windows")] {
            println!("cargo:rustc-link-search=native={}", lib_dir.join("Win64").display());
            println!("cargo:rustc-link-search=native={}", lib_dir.join("Win64/third-party").display());
            println!("cargo:rustc-link-lib=static=libcrypto");
            println!("cargo:rustc-link-lib=static=libssl");
            println!("cargo:rustc-link-lib=static=libsolclient_s");
            // GetUserNameA is in advapi32 — not linked by default with MSVC
            println!("cargo:rustc-link-lib=advapi32");
        } else {
            // From 7.33.x, OpenSSL is embedded in libsolclient.a — no separate ssl/crypto libs.
            println!("cargo:rustc-link-lib=static=solclient");
        }
    }
}
