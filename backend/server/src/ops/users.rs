//! `chakramcp-server users`: create users, list them, set a password or
//! the admin role.

use std::path::PathBuf;

use anyhow::{bail, Result};
use clap::Subcommand;

use chakramcp_app::accounts::{self, NewUser};

use super::{connect, explain, print_table, read_password};

#[derive(Subcommand, Debug)]
pub enum UsersCmd {
    /// Create a user with a personal account.
    Add {
        email: String,
        /// The user's display name.
        #[arg(long)]
        name: String,
        /// Give the user the admin role.
        #[arg(long)]
        admin: bool,
        /// Read the password from stdin (one line) instead of prompting.
        #[arg(long)]
        password_stdin: bool,
    },
    /// List every user with their personal account.
    List {
        /// Print JSON.
        #[arg(long)]
        json: bool,
    },
    /// Give a user the admin role, or take it away with --off.
    SetAdmin {
        email: String,
        #[arg(long)]
        off: bool,
    },
    /// Set a new password for a user.
    SetPassword {
        email: String,
        /// Read the password from stdin (one line) instead of prompting.
        #[arg(long)]
        password_stdin: bool,
    },
}

pub async fn run(config: Option<PathBuf>, cmd: UsersCmd) -> Result<()> {
    let (_, db) = connect(config).await?;
    match cmd {
        UsersCmd::Add {
            email,
            name,
            admin,
            password_stdin,
        } => {
            if accounts::user_exists(&db, &email).await? {
                bail!("a user with the email {email} already exists");
            }
            let password = read_password(password_stdin)?;
            let created = accounts::create_user(
                &db,
                NewUser {
                    email: &email,
                    name: &name,
                    password: &password,
                    is_admin: admin,
                },
            )
            .await
            .map_err(|e| explain(e, String::new))?;
            println!(
                "Created {}{}, with the personal account {}.",
                created.email,
                if created.is_admin { " (admin)" } else { "" },
                created.account_slug,
            );
        }
        UsersCmd::List { json } => {
            let users = accounts::list_users(&db).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&users)?);
            } else if users.is_empty() {
                println!(
                    "No users yet. Create the first admin with:\n  \
                     chakramcp-server users add <email> --name <name> --admin"
                );
            } else {
                let rows: Vec<Vec<String>> = users
                    .into_iter()
                    .map(|u| {
                        vec![
                            u.email,
                            u.name,
                            if u.is_admin { "yes" } else { "" }.to_owned(),
                            u.account.unwrap_or_default(),
                            u.created_at.format("%Y-%m-%d").to_string(),
                        ]
                    })
                    .collect();
                print_table(&["EMAIL", "NAME", "ADMIN", "ACCOUNT", "CREATED"], &rows);
            }
        }
        UsersCmd::SetAdmin { email, off } => {
            accounts::set_admin(&db, &email, !off)
                .await
                .map_err(|e| explain(e, || format!("no user has the email {email}")))?;
            println!(
                "{email} {}. It takes effect at their next sign-in.",
                if off {
                    "is no longer an admin"
                } else {
                    "is now an admin"
                }
            );
        }
        UsersCmd::SetPassword {
            email,
            password_stdin,
        } => {
            if !accounts::user_exists(&db, &email).await? {
                bail!("no user has the email {email}");
            }
            let password = read_password(password_stdin)?;
            accounts::set_password(&db, &email, &password)
                .await
                .map_err(|e| explain(e, || format!("no user has the email {email}")))?;
            println!(
                "Set a new password for {email}. Sign-ins made before this stay \
                 valid until they expire, within 24 hours."
            );
        }
    }
    Ok(())
}
