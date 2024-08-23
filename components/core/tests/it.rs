use std::env;

use assert_fs::{assert::PathAssert, fixture::PathChild, TempDir};
use camino::Utf8Path;
use hinoki_core::{build::build, read_config};
use walkdir::WalkDir;

#[ctor::ctor]
fn init_logging() {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            // Output is only printed for failing tests, but still we shouldn't
            // overload the output with unnecessary info. When debugging a
            // specific test, it's easy to override this default by setting the
            // `RUST_LOG` environment variable.
            //
            // Since tracing_subscriber does prefix matching, the `hinoki=`
            // directive takes effect for `hinoki_core` and any other
            // `hinoki_`-prefixed crates that may be added in the future.
            "hinoki=debug".into()
        }))
        .with(tracing_subscriber::fmt::layer().with_test_writer())
        .init();
}

fn run_test(name: &str, include_drafts: bool) {
    /*
    static OVERWRITE: OnceLock<bool> = OnceLock::new();
    let overwrite = OVERWRITE.get_or_init(|| {
        env::var("HINOKI_TEST_OVERWRITE").is_ok_and(|value| value.to_lowercase() == "true")
    });
    */

    let temp = TempDir::new().unwrap().into_persistent();
    let temp_output_dir = temp.path();

    let tests_dir = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");

    let mut config = read_config(&tests_dir.join(name).join("config.toml")).unwrap();
    config.set_output_dir(Utf8Path::from_path(temp_output_dir).unwrap().to_owned());

    build(config, include_drafts);

    let expected_root = tests_dir.join(format!("{name}.out"));
    let mut expected_iter = WalkDir::new(&expected_root).sort_by_file_name().into_iter();
    let mut actual_iter = WalkDir::new(temp_output_dir).sort_by_file_name().into_iter();

    loop {
        let expected_next = expected_iter.next().transpose().unwrap();
        let actual_next = actual_iter.next().transpose().unwrap();

        if expected_next.is_none() && actual_next.is_none() {
            break;
        }

        let expected_path = expected_next.as_ref().map(|entry| entry.path());
        let actual_path = actual_next.as_ref().map(|entry| entry.path());

        let expected_path_rel = expected_path.map(|p| p.strip_prefix(&expected_root).unwrap());
        let actual_path_rel = actual_path.map(|p| p.strip_prefix(temp_output_dir).unwrap());

        let expected_path_rel = expected_path_rel.unwrap_or_else(|| {
            let missing_file = actual_path_rel.unwrap().display();
            panic!("missing file in output: {missing_file}")
        });
        let actual_path_rel = actual_path_rel.unwrap_or_else(|| {
            let unexpected_file = expected_path_rel.display();
            panic!("found unexpected file in output: {unexpected_file}");
        });

        assert_eq!(expected_path_rel, actual_path_rel);

        if expected_next.as_ref().unwrap().file_type().is_dir()
            && actual_next.as_ref().unwrap().file_type().is_dir()
        {
            continue;
        }

        // FIXME: Replace with something easier to use that also prints diffs
        // in a useful way... Plus allow collecting multiple failed assertions
        // about the actual / expected dir and print all of them, instead of
        // stopping at the first mismatch.
        temp.child(actual_path_rel).assert(predicates::path::eq_file(expected_path.unwrap()));
    }
}

#[test]
fn basic() {
    run_test("basic", true);
}
