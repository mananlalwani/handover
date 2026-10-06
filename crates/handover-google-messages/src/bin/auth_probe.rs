use std::{env, process, thread, time::Duration};

use handover_google_messages::native;
use tokio::io::{stdin, stdout};

fn arguments() -> Result<(String, String), &'static str> {
    let mut args = env::args().skip(1);
    if args.next().as_deref() != Some("--allowed-extension-id") {
        return Err("invalid_arguments");
    }
    let extension_id = args.next().ok_or("invalid_arguments")?;
    let caller_origin = args.next().ok_or("invalid_arguments")?;
    if args.next().is_some()
        || extension_id.len() != 32
        || !extension_id
            .bytes()
            .all(|byte| (b'a'..=b'p').contains(&byte))
    {
        return Err("invalid_arguments");
    }
    Ok((extension_id, caller_origin))
}

#[tokio::main]
async fn main() {
    if env::args()
        .skip(1)
        .eq(["--local-read-only"].map(str::to_owned))
    {
        thread::spawn(|| {
            thread::sleep(Duration::from_secs(30));
            process::exit(124);
        });
        let result = native::run_local_read_only(stdin(), stdout()).await;
        process::exit(if result.is_ok() { 0 } else { 1 });
    }
    let (extension_id, caller_origin) = match arguments() {
        Ok(args) => args,
        Err(code) => {
            eprintln!("{code}");
            process::exit(2);
        }
    };

    // Tokio's stdin reader may be blocked in an OS read that cancellation
    // cannot stop, so enforce the process lifetime outside the runtime.
    thread::spawn(|| {
        thread::sleep(Duration::from_secs(30));
        process::exit(124);
    });

    let result = native::run(stdin(), stdout(), &extension_id, &caller_origin).await;
    process::exit(if result.is_ok() {
        0
    } else {
        eprintln!("host_io_error");
        1
    });
}
