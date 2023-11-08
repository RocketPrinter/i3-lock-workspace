// todo: turn into a proper tui app

use anyhow::Result;
use clap::Parser;
use futures_util::stream::StreamExt;
use notify_rust::{Notification, Urgency};
use std::ops::Deref;
use std::time::Duration;
use tokio::time::sleep;
use tokio::{select, spawn};
use tokio_i3ipc::event::{Event, Subscribe};
use tokio_i3ipc::msg::Msg;
use tokio_i3ipc::reply::Workspace;
use tokio_i3ipc::I3;

/// Locks i3 to a single or multiple workspaces for a set period of time to force yourself to focus on work
#[derive(Parser)]
#[command(author, version)]
struct Cli {
    /// How long to lock the workspace for (ex: "1h 15min 3s")
    duration: humantime::Duration,
    /// Name of workspace or workspaces to lock on, uses the current one if not set
    workspaces: Vec<String>,
    /// Start after a delay.
    #[arg(short, long)]
    delay: Option<humantime::Duration>,
    /// Invert the selection aka allow every workspace that's not in the list.
    #[arg(short, long)]
    invert: bool,
    /// Use workspace numbers instead of names
    #[arg(short = 'n', long)]
    use_numbers: bool,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let mut cli = Cli::parse();

    let mut i3 = I3::connect().await?;

    // if no workspaces were detect the currently focused one
    if cli.workspaces.is_empty() {
        let ws = get_focused_workspace(&mut i3)
            .await?
            .expect("No workspace is focused!");

        println!("Locking on workspace {}", ws.name);
        cli.workspaces.push(ws.name);
    }

    // delay
    if let Some(delay) = cli.delay {
        println!("Starting after {delay}");
        if delay.as_secs() > 15 {
            sleep(*delay.deref() - Duration::from_secs(10)).await;

            Notification::new()
                .summary("Workspaces locking in 10 seconds!")
                .auto_icon()
                .timeout(10_000)
                .urgency(Urgency::Critical)
                .show_async()
                .await?;

            sleep(Duration::from_secs(10)).await;
        } else {
            sleep(delay.into()).await;
        }
    }

    // we check if the current workspace is allowed
    {
        let current_ws = if cli.use_numbers {
            get_focused_workspace(&mut i3)
                .await?
                .map(|ws| ws.num.to_string())
        } else {
            get_focused_workspace(&mut i3).await?.map(|ws| ws.name)
        };
        change_workspace_if_disallowed(
            current_ws.as_deref(),
            &mut i3,
            cli.workspaces.as_slice(),
            cli.invert,
            cli.use_numbers,
        )
        .await?;
    }

    println!("Started!");

    let mut sleep = spawn(sleep(cli.duration.into()));

    // dedicated connection for listening
    let mut listener = I3::connect().await?;
    listener.subscribe([Subscribe::Workspace]).await?;
    let mut listener = listener.listen();

    // we listen for workspace changes
    loop {
        select! {
            _ = &mut sleep => {break;},

            event = listener.next() => {
                let Event::Workspace(event) = event.expect("i3-ipc connection was closed!")? else {continue};
                let identifier = if cli.use_numbers {
                    event.current.and_then(|node|node.num.map(|n|n.to_string()))
                } else {
                    event.current.and_then(|node|node.name)
                };
                change_workspace_if_disallowed(identifier.as_deref(), &mut i3, cli.workspaces.as_slice(), cli.invert, cli.use_numbers).await?;
            }
        }
    }

    Notification::new()
        .summary("Workspaces unlocked")
        .auto_icon()
        .timeout(2000)
        .show_async()
        .await?;

    Ok(())
}

async fn get_focused_workspace(i3: &mut I3) -> Result<Option<Workspace>> {
    Ok(i3.get_workspaces().await?.into_iter().find(|ws| ws.focused))
}

async fn change_workspace_if_disallowed(
    current_workspace: Option<&str>,
    i3: &mut I3,
    allowed_workspaces: &[String],
    invert: bool,
    numbers: bool,
) -> Result<()> {
    if invert
        ^ !allowed_workspaces
            .iter()
            .any(|allowed| Some(allowed.as_str()) == current_workspace)
    {
        println!("yes");
        let command_prefix = if numbers {
            "workspace number"
        } else {
            "workspace"
        };
        i3.send_msg_body(
            Msg::RunCommand,
            format!("{} {}", command_prefix, allowed_workspaces[0]),
        )
        .await?;
        Notification::new()
            .summary("Workspace locked!")
            .auto_icon()
            .timeout(2_000)
            .urgency(Urgency::Critical)
            .show_async()
            .await?;
    }
    Ok(())
}
