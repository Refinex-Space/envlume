mod entry;

use envlume_application::ErrorCode;
use envlume_storage::StorePaths;
use std::process::ExitCode;

fn main() -> ExitCode {
    // T05 supplies the production workspace resolver. Test registrations are
    // injected by the subprocess test and cannot be enabled by CLI arguments.
    entry::run(
        &std::env::args_os().skip(1).collect::<Vec<_>>(),
        StorePaths::local,
        |_| Err(ErrorCode::ProjectNotTrusted),
    )
}
