use vergen::{BuildBuilder, CargoBuilder, Emitter, RustcBuilder};
use vergen_git2::Git2Builder;

macro_rules! emit_instructions {
    ($build:expr, $cargo:expr, $rustc:expr $(, $git2:expr)?) => {
        {
            let mut emitter = Emitter::default();
            emitter.add_instructions($build)?;
            emitter.add_instructions($cargo)?;
            emitter.add_instructions($rustc)?;
            $(emitter.add_instructions($git2)?;)?
            emitter.emit()?;
        }
    };
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Check for reproducible build environment variables
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    println!("cargo:rerun-if-env-changed=VERGEN_IDEMPOTENT");

    // Configure build instructions - respect reproducible build settings
    let build = if std::env::var_os("VERGEN_IDEMPOTENT").is_some() {
        // For reproducible builds, only include non-temporal information
        BuildBuilder::default()
            .build_date(false)
            .build_timestamp(false)
            .build()?
    } else {
        // Normal builds include timestamps (vergen will respect SOURCE_DATE_EPOCH)
        BuildBuilder::all_build()?
    };
    let cargo = CargoBuilder::all_cargo()?;
    let rustc = RustcBuilder::all_rustc()?;

    // Get git instructions if we're in a git repository and not in idempotent mode
    if std::path::Path::new(".git").exists() && std::env::var_os("VERGEN_IDEMPOTENT").is_none() {
        if let Ok(git2) = Git2Builder::default()
            .branch(true)
            .commit_author_email(true)
            .commit_author_name(true)
            .commit_count(true)
            .commit_message(true)
            .commit_timestamp(true)
            .describe(true, true, None)
            .sha(true)
            .build()
        {
            emit_instructions!(&build, &cargo, &rustc, &git2);
        } else {
            emit_instructions!(&build, &cargo, &rustc);
        }
    } else {
        emit_instructions!(&build, &cargo, &rustc);
    }

    Ok(())
}
