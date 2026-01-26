use surrealdb::Connection;

use surrealfs::SurrealFs;

pub mod curl;
mod fs_ops;
mod ls;
mod shell;
pub mod util;

pub struct ReplState<DB: Connection> {
    pub fs: SurrealFs<DB>,
    pub cwd: String,
}

pub enum ReplControl {
    Continue,
    Exit,
}

pub async fn dispatch<DB>(
    cmd: &str,
    args: &[&str],
    state: &mut ReplState<DB>,
) -> surrealfs::Result<ReplControl>
where
    DB: Connection,
{
    let outcome = match cmd {
        "ls" => ls::run(args, state).await.map(|_| ReplControl::Continue),
        "cat" => fs_ops::cat(args, state)
            .await
            .map(|_| ReplControl::Continue),
        "tail" => fs_ops::tail(args, state)
            .await
            .map(|_| ReplControl::Continue),
        "nl" => fs_ops::nl(args, state).await.map(|_| ReplControl::Continue),
        "grep" => fs_ops::grep(args, state)
            .await
            .map(|_| ReplControl::Continue),
        "touch" => fs_ops::touch(args, state)
            .await
            .map(|_| ReplControl::Continue),
        "edit" => fs_ops::edit(args, state)
            .await
            .map(|_| ReplControl::Continue),
        "mkdir" => fs_ops::mkdir(args, state)
            .await
            .map(|_| ReplControl::Continue),
        "write_file" => fs_ops::write_file(args, state)
            .await
            .map(|_| ReplControl::Continue),
        "cp" => fs_ops::cp(args, state).await.map(|_| ReplControl::Continue),
        "curl" => curl::run(args, state).await.map(|_| ReplControl::Continue),
        "pwd" => shell::pwd(state).map(|_| ReplControl::Continue),
        "cd" => shell::cd(args, state).await.map(|_| ReplControl::Continue),
        "help" => {
            shell::print_help();
            Ok(ReplControl::Continue)
        }
        "exit" | "quit" => Ok(ReplControl::Exit),
        _ => {
            shell::print_help();
            Ok(ReplControl::Continue)
        }
    }?;

    Ok(outcome)
}
