extern crate bindgen;
use std::sync::Arc;
use std::{env, io::Write, path::PathBuf};
use ureq::Agent;

// Tarball filename — used as the local download filename.
#[cfg(target_os = "windows")]
const SOLCLIENT_ARCHIVE_PATH: &str = "solclient_Win_vs2015_7.33.2.3.tar.gz";

#[cfg(target_os = "macos")]
const SOLCLIENT_ARCHIVE_PATH: &str = "solclient_Darwin-universal2_opt_7.33.2.3.tar.gz";

#[cfg(all(target_os = "linux", target_arch = "x86_64", not(target_env = "musl")))]
const SOLCLIENT_ARCHIVE_PATH: &str = "solclient_Linux26-x86_64_opt_7.33.2.3.tar.gz";

#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const SOLCLIENT_ARCHIVE_PATH: &str = "solclient_Linux-aarch64_opt_7.33.2.3.tar.gz";

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "musl"))]
const SOLCLIENT_ARCHIVE_PATH: &str = "solclient_Linux_musl-x86_64_opt_7.33.2.3.tar.gz";

// Official Solace download URLs — all resolve to .tar.gz tarballs without authentication.
#[cfg(target_os = "windows")]
const SOLCLIENT_OFFICIAL_URL: &str = "https://products.solace.com/download/C_API_VS2015";

#[cfg(target_os = "macos")]
const SOLCLIENT_OFFICIAL_URL: &str = "https://products.solace.com/download/C_API_OSX";

#[cfg(all(target_os = "linux", target_arch = "x86_64", not(target_env = "musl")))]
const SOLCLIENT_OFFICIAL_URL: &str = "https://products.solace.com/download/C_API_LINUX64";

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "musl"))]
const SOLCLIENT_OFFICIAL_URL: &str = "https://products.solace.com/download/C_API_MUSL";

// Linux aarch64 has no known official URL — users must set SOLCLIENT_TARBALL_URL or SOLCLIENT_LIB_PATH.
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const SOLCLIENT_OFFICIAL_URL: &str = "";

fn build_ureq_agent() -> Agent {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    let mut root_store = rustls::RootCertStore::empty();
    for cert in rustls_native_certs::load_native_certs().expect("could not load platform certs") {
        root_store.add(cert).unwrap();
    }
    let tls_config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    ureq::builder().tls_config(Arc::new(tls_config)).build()
}
fn download_and_unpack(url: &str, tarball_path: PathBuf, tarball_unpack_path: PathBuf) {
    let mut content = Vec::new();
    build_ureq_agent()
        .get(url)
        .call()
        .unwrap()
        .into_reader()
        .read_to_end(&mut content)
        .unwrap();

    let mut file_gz = std::fs::File::create(tarball_path.clone()).unwrap();
    file_gz.write_all(&content).unwrap();
    file_gz.sync_data().unwrap();

    let file_gz = std::fs::File::open(tarball_path).unwrap();
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file_gz));
    archive
        .entries()
        .unwrap()
        .filter_map(|r| r.ok())
        .map(|mut entry| -> std::io::Result<PathBuf> {
            let strip_path = entry.path()?.iter().skip(1).collect::<std::path::PathBuf>();
            let path = tarball_unpack_path.join(strip_path);
            entry.unpack(&path)?;
            Ok(path)
        })
        .filter_map(|e| e.ok())
        .for_each(|x| println!("> {}", x.display()));
}

fn main() {
    // do nothing if we are just building the docs
    if std::env::var("DOCS_RS").is_ok() {
        return;
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    let solclient_folder_name = "solclient-7.33.2.3";
    let solclient_folder_path = out_dir.join(solclient_folder_name);

    let lib_dir = if env::var("SOLCLIENT_LIB_PATH").is_ok() {
        PathBuf::from(env::var("SOLCLIENT_LIB_PATH").unwrap())
    } else {
        // Treat an empty SOLCLIENT_TARBALL_URL (e.g. an unset GitHub Actions secret
        // that gets expanded to "") the same as the variable not being set at all.
        // Only panic if neither SOLCLIENT_TARBALL_URL nor an official URL is available.
        let solclient_tarball_url = env::var("SOLCLIENT_TARBALL_URL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                if SOLCLIENT_OFFICIAL_URL.is_empty() {
                    panic!(
                        "No official download URL is known for this platform (Linux aarch64).\n\
                         Set SOLCLIENT_TARBALL_URL to a URL pointing to the Solace C API {solclient_folder_name} \
                         tarball for your platform, or set SOLCLIENT_LIB_PATH to a directory containing \
                         the pre-extracted library files."
                    );
                }
                SOLCLIENT_OFFICIAL_URL.to_string()
            });

        let solclient_tarball_path = out_dir.join(SOLCLIENT_ARCHIVE_PATH);

        if !solclient_folder_path.is_dir() {
            eprintln!(
                "Solclient not found. Downloading from {}",
                solclient_tarball_url
            );
            download_and_unpack(
                &solclient_tarball_url,
                solclient_tarball_path,
                solclient_folder_path.clone(),
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
            println!("cargo:rustc-link-lib=static=libcrypto_s");
            println!("cargo:rustc-link-lib=static=libssl_s");
            println!("cargo:rustc-link-lib=static=libsolclient_s");
        } else {
            // From 7.33.x, OpenSSL is embedded in libsolclient.a — no separate ssl/crypto libs.
            println!("cargo:rustc-link-lib=static=solclient");
        }
    }
}
