use lyra_downloads_core::i18n::{init, tr, trf};
use std::path::Path;
use std::process::Command;

// Each child owns gettext's process-global locale. Never mutate it in a
// multithreaded test process or depend on installed system catalogs.
#[test]
fn catalog_runtime_child() {
    let Ok(expected) = std::env::var("LYRA_TEST_TRANSLATION") else {
        return;
    };
    init();
    assert_eq!(tr("Download finished"), expected);
    let formatted = trf("Could not resume: {e}", &[("e", "TEST_DETAIL")]);
    assert!(formatted.ends_with("TEST_DETAIL"));
    assert!(!formatted.contains("{e}"));
    assert_eq!(tr("Uncatalogued message"), "Uncatalogued message");
}

#[test]
fn real_catalogs_and_english_fallback() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = tempfile::tempdir().unwrap();
    for locale in ["en_US", "pt_BR", "es_ES"] {
        let output = dir
            .path()
            .join(locale)
            .join("LC_MESSAGES/lyra-downloads.mo");
        std::fs::create_dir_all(output.parent().unwrap()).unwrap();
        assert!(Command::new("msgfmt")
            .args(["--check", "-o"])
            .arg(output)
            .arg(root.join(format!("po/{locale}.po")))
            .status()
            .unwrap()
            .success());
    }
    for (locale, expected) in [
        ("en_US", "Download finished"),
        ("pt_BR", "Download concluído"),
        ("es_ES", "Descarga completada"),
        ("fr_FR", "Download finished"),
    ] {
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "catalog_runtime_child", "--nocapture"])
            .env("LC_ALL", "en_US.UTF-8")
            .env("LANGUAGE", locale)
            .env("LYRA_DOWNLOADS_LOCALEDIR", dir.path())
            .env("LYRA_TEST_TRANSLATION", expected)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{locale}: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
