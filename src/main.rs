// todo: turn into a proper tui app

use anyhow::Result;
use clap::Parser;
use futures_util::stream::StreamExt;
use notify_rust::{Notification, Urgency};
use std::ops::{Deref, Not};
use std::time::Duration;
use tokio::time::sleep;
use tokio::{select, spawn};
use tokio_i3ipc::event::{Event, Subscribe, WorkspaceChange, WorkspaceData};
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

    // if workspaces is empty we detect the currently focused one
    if cli.workspaces.is_empty() {
        let ws = get_focused_workspace(&mut i3)
            .await?
            .expect("No workspace is focused!");

        println!("Locking on workspace {}", ws.name);
        cli.workspaces.push(ws.name);
    }

    // if invert is true then we can't easily find an allowed workspace
    let mut last_allowed_workspace = cli.invert.not().then(|| cli.workspaces[0].clone());

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
        let current_ws = get_focused_workspace(&mut i3).await?.map(|ws| {
            if cli.use_numbers {
                ws.num.to_string()
            } else {
                ws.name
            }
        });
        change_workspace_if_disallowed(
            current_ws.as_deref(),
            &mut last_allowed_workspace,
            &mut i3,
            &cli,
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
                // we make sure it's a Focus change otherwise we might get inconsistent behaviour
                let WorkspaceData { current, change: WorkspaceChange::Focus , old:_ } = *event else {continue};
                let identifier = if cli.use_numbers {
                    current.and_then(|node|node.num.map(|n|n.to_string()))
                } else {
                    current.and_then(|node|node.name)
                };
                change_workspace_if_disallowed(identifier.as_deref(), &mut last_allowed_workspace , &mut i3, &cli).await?;
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

fn is_workspace_dissalowed(workspace: &str, cli: &Cli) -> bool {
    cli.invert ^ !cli.workspaces.iter().any(|allowed| allowed == workspace)
}

async fn change_workspace_if_disallowed(
    current_workspace: Option<&str>,
    last_allowed_workspace: &mut Option<String>,
    i3: &mut I3,
    cli: &Cli,
) -> Result<()> {
    if current_workspace.is_none() || is_workspace_dissalowed(current_workspace.unwrap(), cli) {
        if let Some(next_workspace) = last_allowed_workspace {
            // we switch to the last allowed workspace
            let command_prefix = if cli.use_numbers {
                "workspace number"
            } else {
                "workspace"
            };
            i3.send_msg_body(
                Msg::RunCommand,
                format!("{} {}", command_prefix, next_workspace),
            )
            .await?;
        } else {
            // we don't have an allowed workspace to switch to so the simplest solution is to open a temporary one
            i3.send_msg_body(Msg::RunCommand, format!("{} {}", "workspace", "temp"))
                .await?;
        }
        Notification::new()
            .summary("Workspace locked!")
            .auto_icon()
            .timeout(2_000)
            .urgency(Urgency::Critical)
            .show_async()
            .await?;
    } else {
        // this workspace is allowed
        *last_allowed_workspace = Some(current_workspace.unwrap().to_string());
    }
    Ok(())
}
