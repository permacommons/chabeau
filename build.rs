use vergen::{BuildBuilder, CargoBuilder, Emitter, RustcBuilder};
use vergen_git2::Git2Builder;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let build = BuildBuilder::all_build()?;
    let cargo = CargoBuilder::all_cargo()?;
    let rustc = RustcBuilder::all_rustc()?;

    // Try to configure git2, but don't fail if git is not available (e.g., crates.io builds)
    let git2_result = Git2Builder::default()
        .branch(true)
        .commit_author_email(true)
        .commit_author_name(true)
        .commit_count(true)
        .commit_message(true)
        .commit_timestamp(true)
        .describe(true, true, None) // enable describe, include tags, no match pattern
        .sha(true)
        .build();

    // Only add git instructions if git is available
    if let Ok(git2) = git2_result {
        Emitter::default()
            .add_instructions(&build)?
            .add_instructions(&cargo)?
            .add_instructions(&rustc)?
            .add_instructions(&git2)?
            .emit()?;
    } else {
        // Set fallback values when git is not available
        println!("cargo:rustc-env=VERGEN_GIT_BRANCH=unknown");
        println!("cargo:rustc-env=VERGEN_GIT_COMMIT_AUTHOR_EMAIL=unknown");
        println!("cargo:rustc-env=VERGEN_GIT_COMMIT_AUTHOR_NAME=unknown");
        println!("cargo:rustc-env=VERGEN_GIT_COMMIT_COUNT=0");
        println!("cargo:rustc-env=VERGEN_GIT_COMMIT_MESSAGE=unknown");
        println!("cargo:rustc-env=VERGEN_GIT_COMMIT_TIMESTAMP=unknown");
        println!("cargo:rustc-env=VERGEN_GIT_DESCRIBE=unknown");
        println!("cargo:rustc-env=VERGEN_GIT_SHA=unknown");

        Emitter::default()
            .add_instructions(&build)?
            .add_instructions(&cargo)?
            .add_instructions(&rustc)?
            .emit()?;
    }

    Ok(())
}
