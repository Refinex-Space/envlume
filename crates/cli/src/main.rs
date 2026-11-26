use envlume_storage::StorePaths;
fn main() -> std::process::ExitCode {
    envlume::run(
        &std::env::args_os().skip(1).collect::<Vec<_>>(),
        StorePaths::local,
        |id| envlume::resolve(StorePaths::local(), id),
    )
}
