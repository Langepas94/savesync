fn main() {
    // M1: каркас. Демон получит sync engine, IPC-сервер (unix socket /
    // named pipe, JSON-RPC) и бэкенды в последующих feature-ветках.
    eprintln!(
        "savesync-daemon {} (skeleton, see docs/plan.md)",
        env!("CARGO_PKG_VERSION")
    );
}
