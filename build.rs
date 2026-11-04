use std::{env, fs, path::Path};

// Embed the committed Next.js export so release binaries still need no Node
// installation, network access, or files alongside the executable.
fn main() {
    let root = env::var("CARGO_MANIFEST_DIR").unwrap();
    let assets = Path::new(&root).join("assets/mobile");
    println!("cargo:rerun-if-changed=assets/mobile");
    let mut entries = Vec::new();
    collect(&assets.join("_next/static"), &assets, &mut entries);
    entries.sort();
    let mut source = String::from(
        "pub fn asset(path: &str) -> Option<(&'static [u8], &'static str)> { match path {\n",
    );
    for (url, file) in entries {
        let mime = match Path::new(&file).extension().and_then(|s| s.to_str()) {
            Some("js") => "text/javascript; charset=utf-8",
            Some("css") => "text/css; charset=utf-8",
            Some("woff2") => "font/woff2",
            Some("woff") => "font/woff",
            Some("png") => "image/png",
            Some("svg") => "image/svg+xml",
            _ => "application/octet-stream",
        };
        source.push_str(&format!(
            "{url:?} => Some((include_bytes!({file:?}), {mime:?})),\n"
        ));
    }
    source.push_str("_ => None, } }\n");
    fs::write(
        Path::new(&env::var("OUT_DIR").unwrap()).join("mobile_assets.rs"),
        source,
    )
    .unwrap();
}

fn collect(dir: &Path, root: &Path, entries: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir)
        .expect("Missing mobile export. Run npm ci && npm run bundle in workbench-mobile.")
    {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(&path, root, entries);
        } else {
            entries.push((
                format!("/{}", path.strip_prefix(root).unwrap().to_str().unwrap()),
                path.to_str().unwrap().to_string(),
            ));
        }
    }
}
