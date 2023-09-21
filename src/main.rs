// todo: polish and publish

use clap::Parser;
use tokio_i3ipc::I3;
use anyhow::Result;
use tokio_i3ipc::event::{Event, Subscribe};
use futures_util::stream::StreamExt;
use notify_rust::{Notification, Urgency};
use tokio::{select, spawn};
use tokio::time::sleep;
use tokio_i3ipc::msg::Msg;

/// Locks i3 to a single or multiple workspaces for a set period of time to force yourself to focus on work
#[derive(Parser)]
#[command(author, version)]
struct Cli {
    /// How long to lock the workspace for (ex: "1h 15min 3s")
    duration: humantime::Duration,
    /// Workspace or workspaces to lock on, uses the curent one if not set
    workspaces: Vec<String>,
    /// Start after a delay.
    #[arg(short,long)]
    delay: Option<humantime::Duration>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let mut cli = Cli::parse();

    let mut i3 = I3::connect().await?;

    // if no workspaces were detect the currently focused one
    if cli.workspaces.is_empty() {
        let name = get_focused_workspace(&mut i3).await?.expect("No workspace is focused!");

        println!("Locking on workspace {name}");
        cli.workspaces.push(name);
    }

    // delay
    if let Some(delay) = cli.delay {
        println!("Starting after {delay}");
        sleep(delay.into()).await;
    }

    // we check if the current workspace is allowed
    {
        let current_ws = get_focused_workspace(&mut i3).await?;
        change_workspace_if_disallowed(current_ws.as_deref(), &mut i3, cli.workspaces.as_slice()).await?;
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
                let name = event.current.and_then(|node|node.name);
                change_workspace_if_disallowed(name.as_deref(), &mut i3, cli.workspaces.as_slice()).await?;
            }
        }

    }

    Notification::new()
        .summary("Workspaces unlocked")
        .auto_icon()
        .timeout(2000)
        .show_async().await?;

    Ok(())
}

async fn get_focused_workspace(i3: &mut I3) -> Result<Option<String>> {
    Ok(
        i3.get_workspaces().await?.into_iter()
            .find_map(|ws|ws.focused.then_some(ws.name))
    )
}

async fn change_workspace_if_disallowed(current_workspace: Option<&str>, i3: &mut I3, allowed_workspaces: &[String]) -> Result<()> {
    if !allowed_workspaces.iter().any(|allowed| Some(allowed.as_str()) == current_workspace) {
        i3.send_msg_body(Msg::RunCommand, format!("workspace {}", allowed_workspaces[0]) ).await?;
        Notification::new()
            .summary("Workspace locked!")
            .auto_icon()
            .timeout(2000)
            .urgency(Urgency::Critical)
            .show_async().await?;
    }
    Ok(())
}