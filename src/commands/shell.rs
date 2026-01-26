use surrealdb::Connection;

use surrealfs::FsError;

use super::ReplState;
use super::util::{help_error, resolve_cli_path};

pub fn pwd<DB>(state: &ReplState<DB>) -> Result<(), FsError>
where
    DB: Connection,
{
    println!("{}", state.cwd);
    Ok(())
}

pub async fn cd<DB>(args: &[&str], state: &mut ReplState<DB>) -> Result<(), FsError>
where
    DB: Connection,
{
    match args {
        [path] => {
            let target = resolve_cli_path(&state.cwd, path);
            match state.fs.cd(&state.cwd, &target).await {
                Ok(new_cwd) => {
                    state.cwd = new_cwd;
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        _ => Err(help_error()),
    }
}

pub fn print_help() {
    println!("Commands:");
    println!("  ls [options] [path]");
    println!("     options: -l (long), -a (all), -R (recursive), -d (dir only), -h (human sizes)");
    println!("  cat <path>");
    println!("  tail [n] <path>");
    println!("  read <path> <offset> <limit>");
    println!("  nl <path> [start]");
    println!("  grep [-r|--recursive] <pattern> <path>");
    println!("  touch <path>");
    println!("  edit <path> <old> <new> [replace_all]");
    println!("  mkdir [-p] <path>");
    println!("  write_file <path> <content>");
    println!("  cp <src> <dest>");
    println!("  curl [options] <url>");
    println!("     options: -o <file>, -O, -L, -H <h:v>, -d <data>, -X <method>, > <file>");
    println!("     pipeline: curl <url> | write_file <path>");
    println!("  pwd");
    println!("  cd <path>");
    println!("  help");
    println!("  exit | quit");
}
