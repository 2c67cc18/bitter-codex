use codex_test_binary_support::TestBinaryDispatchGuard;
use codex_test_binary_support::TestBinaryDispatchMode;
use codex_test_binary_support::configure_test_binary_dispatch;
use ctor::ctor;

#[ctor]
pub static CODEX_ALIASES_TEMP_DIR: Option<TestBinaryDispatchGuard> = {
    configure_test_binary_dispatch("codex-core-tests", |_exe_name, _argv1| {
        TestBinaryDispatchMode::InstallAliases
    })
};

mod cli_stream;
mod live_cli;
