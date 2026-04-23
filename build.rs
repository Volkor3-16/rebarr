fn main() {
    println!("cargo:rerun-if-env-changed=GIT_COMMIT");
    if let Ok(sha) = std::env::var("GIT_COMMIT") {
        println!("cargo:rustc-env=GIT_COMMIT_HASH={}", sha);
    }
}
