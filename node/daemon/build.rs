fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let dist = std::path::Path::new(&manifest_dir).join("../../ui/web/dist");

    // If the UI hasn't been built yet, create a placeholder so include_dir! doesn't panic.
    if !dist.exists() {
        std::fs::create_dir_all(&dist).expect("create ui/web/dist placeholder");
        std::fs::write(
            dist.join("index.html"),
            concat!(
                "<!DOCTYPE html><html><head><title>Howm</title></head><body style='font-family:sans-serif;padding:2rem'>",
                "<h2>UI not built</h2>",
                "<p>Run <code>cd ui/web &amp;&amp; npm run build</code> then rebuild the daemon.</p>",
                "</body></html>"
            ),
        )
        .expect("write placeholder index.html");
    }

    // Re-run if the UI source or dist changes
    println!("cargo:rerun-if-changed=../../ui/web/dist");
    println!("cargo:rerun-if-changed=../../ui/web/src");
    println!("cargo:rerun-if-changed=../../ui/web/public");
}
