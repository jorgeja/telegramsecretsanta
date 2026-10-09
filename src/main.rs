mod santa;
mod store;

use std::sync::Arc;

use teloxide::{
    prelude::*,
    types::{Me, ParseMode},
    utils::{command::BotCommands, html},
};

use store::{Participant, Store};

/// Fewer than this and everyone can work out who their Santa is.
const MIN_PARTICIPANTS: usize = 3;

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "🎅 Secret Santa bot commands:")]
enum Command {
    /// Show this help.
    Help,
    /// Start a private chat with the bot (needed before registering).
    Start,
    /// Join this group's Secret Santa.
    #[command(alias = "join")]
    Register,
    /// Leave this group's Secret Santa.
    #[command(alias = "leave")]
    Unregister,
    /// Show who has registered.
    List,
    /// Draw names and DM everyone their match (admins only).
    Draw,
    /// Clear registrations and matches to start over (admins only).
    Reset,
    /// Get your match sent to you again in a private message.
    MySanta,
}

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    let state_file = std::env::var("SANTA_STATE_FILE").unwrap_or_else(|_| "santa_state.json".into());
    let store = Arc::new(Store::load(&state_file).expect("failed to load state file"));
    log::info!("Starting Secret Santa bot, saving state to {state_file}");

    let bot = Bot::from_env();
    if let Err(e) = bot.set_my_commands(Command::bot_commands()).await {
        log::warn!("Couldn't register the command menu: {e}");
    }

    let handler = Update::filter_message().filter_command::<Command>().endpoint(handle_command);

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![store])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
}

async fn handle_command(bot: Bot, msg: Message, me: Me, cmd: Command, store: Arc<Store>) -> ResponseResult<()> {
    let is_group = msg.chat.is_group() || msg.chat.is_supergroup();
    match cmd {
        Command::Help => reply(&bot, &msg, &Command::descriptions().to_string()).await,
        Command::Start if !is_group => reply(&bot, &msg, &welcome_text()).await,
        Command::Start => reply(&bot, &msg, &Command::descriptions().to_string()).await,
        Command::MySanta => my_santa(&bot, &msg, &store).await,
        _ if !is_group => reply(&bot, &msg, "This command only works in a group chat.").await,
        Command::Register => register(&bot, &msg, &me, &store).await,
        Command::Unregister => unregister(&bot, &msg, &store).await,
        Command::List => list(&bot, &msg, &store).await,
        Command::Draw => draw(&bot, &msg, &store).await,
        Command::Reset => reset(&bot, &msg, &store).await,
    }
}

async fn register(bot: &Bot, msg: &Message, me: &Me, store: &Store) -> ResponseResult<()> {
    let Some(user) = &msg.from else { return Ok(()) };
    let participant = Participant::from_user(user);

    let precheck = store.read(msg.chat.id, |game| match game {
        Some(game) if game.is_drawn() => Some("Names have already been drawn for this round."),
        Some(game) if game.is_registered(user.id) => Some("You're already registered! 🎄"),
        _ => None,
    });
    if let Some(text) = precheck {
        return reply(bot, msg, text).await;
    }

    // Bots can only message people who have opened a private chat with them, so make sure
    // we can reach this person now rather than finding out when names are drawn.
    let title = group_title(msg);
    let confirmation = format!(
        "👋 You're in the Secret Santa for <b>{}</b>! I'll send your match here once names are drawn.",
        html::escape(&title)
    );
    if bot.send_message(user.id, confirmation).parse_mode(ParseMode::Html).await.is_err() {
        let text = format!(
            "{}, I can't message you privately yet. Open {} and press <b>Start</b>, then send /register here again.",
            participant.display(),
            html::escape(&format!("https://t.me/{}", me.username()))
        );
        return reply(bot, msg, &text).await;
    }

    let result = store.update(msg.chat.id, |game| {
        game.title = title;
        if game.is_drawn() {
            Err("Names have already been drawn for this round.")
        } else if game.is_registered(user.id) {
            Err("You're already registered! 🎄")
        } else {
            game.participants.push(participant.clone());
            Ok(game.participants.len())
        }
    });
    match result {
        Ok(count) => {
            let text = format!("🎄 {} joined the Secret Santa! ({count} registered)", participant.display());
            reply(bot, msg, &text).await
        }
        Err(text) => reply(bot, msg, text).await,
    }
}

async fn unregister(bot: &Bot, msg: &Message, store: &Store) -> ResponseResult<()> {
    let Some(user) = &msg.from else { return Ok(()) };
    let result = store.update(msg.chat.id, |game| {
        if game.is_drawn() {
            return Err("Names have already been drawn, so you can't leave this round. An admin can /reset it.");
        }
        let before = game.participants.len();
        game.participants.retain(|p| p.id != user.id);
        if game.participants.len() == before {
            return Err("You weren't registered.");
        }
        Ok(game.participants.len())
    });
    match result {
        Ok(count) => {
            let name = Participant::from_user(user).display();
            reply(bot, msg, &format!("👋 {name} left the Secret Santa. ({count} registered)")).await
        }
        Err(text) => reply(bot, msg, text).await,
    }
}

async fn list(bot: &Bot, msg: &Message, store: &Store) -> ResponseResult<()> {
    let text = store.read(msg.chat.id, |game| match game {
        Some(game) if !game.participants.is_empty() => {
            let names: Vec<String> = game.participants.iter().map(|p| format!("• {}", p.display())).collect();
            let status = if game.is_drawn() {
                "Names have been drawn! 🎁"
            } else {
                "Registration is open — use /register to join."
            };
            format!(
                "🎅 <b>Secret Santa participants ({})</b>\n{}\n\n{status}",
                names.len(),
                names.join("\n")
            )
        }
        _ => "Nobody has registered yet. Use /register to join!".to_string(),
    });
    reply(bot, msg, &text).await
}

async fn draw(bot: &Bot, msg: &Message, store: &Store) -> ResponseResult<()> {
    if !is_admin(bot, msg).await? {
        return reply(bot, msg, "Only group admins can draw names.").await;
    }

    let result = store.update(msg.chat.id, |game| {
        if game.is_drawn() {
            return Err("Names have already been drawn. Use /reset to start a new round.".to_string());
        }
        if game.participants.len() < MIN_PARTICIPANTS {
            return Err(format!(
                "Need at least {MIN_PARTICIPANTS} participants to draw names (currently {}).",
                game.participants.len()
            ));
        }
        let pairs = santa::draw(&game.participants, &mut rand::rng());
        game.assignments = Some(pairs.iter().map(|(giver, receiver)| (giver.id, receiver.id)).collect());
        Ok((game.title.clone(), pairs))
    });
    let (title, pairs) = match result {
        Ok(drawn) => drawn,
        Err(text) => return reply(bot, msg, &text).await,
    };

    let mut unreachable = Vec::new();
    for (giver, receiver) in &pairs {
        if let Err(e) = bot.send_message(giver.id, match_text(&title, receiver)).parse_mode(ParseMode::Html).await {
            log::warn!("Couldn't DM {}: {e}", giver.id);
            unreachable.push(giver.display());
        }
    }

    let mut text = format!(
        "🎁 Names have been drawn for {} people! Check your private messages from me.",
        pairs.len()
    );
    if !unreachable.is_empty() {
        text.push_str(&format!(
            "\n\n⚠️ I couldn't message: {}.\nOpen a private chat with me, press Start, and send /mysanta to get your match.",
            unreachable.join(", ")
        ));
    }
    reply(bot, msg, &text).await
}

async fn reset(bot: &Bot, msg: &Message, store: &Store) -> ResponseResult<()> {
    if !is_admin(bot, msg).await? {
        return reply(bot, msg, "Only group admins can reset the Secret Santa.").await;
    }
    store.remove(msg.chat.id);
    reply(bot, msg, "🔄 Secret Santa has been reset. Use /register to join the new round!").await
}

async fn my_santa(bot: &Bot, msg: &Message, store: &Store) -> ResponseResult<()> {
    let Some(user) = &msg.from else { return Ok(()) };
    let matches = store.matches_for(user.id);

    let text = if matches.is_empty() {
        "You don't have a Secret Santa match yet. Register in your group with /register and wait for the draw!"
            .to_string()
    } else {
        matches
            .iter()
            .map(|(title, receiver)| match_text(title, receiver))
            .collect::<Vec<_>>()
            .join("\n\n")
    };

    // Never reveal a match in the group itself.
    let sent = bot.send_message(user.id, text).parse_mode(ParseMode::Html).await;
    if msg.chat.is_private() {
        sent?;
        return Ok(());
    }
    match sent {
        Ok(_) => reply(bot, msg, "📬 Sent you a private message!").await,
        Err(_) => reply(bot, msg, "I can't message you privately yet — open a chat with me, press Start, and try again.").await,
    }
}

fn match_text(group_title: &str, receiver: &Participant) -> String {
    format!(
        "🎅 <b>Secret Santa — {}</b>\n\nYou're buying a gift for: <b>{}</b> 🎁\n\nShh, keep it a secret! 🤫",
        html::escape(group_title),
        receiver.display()
    )
}

fn welcome_text() -> String {
    format!(
        "🎅 Ho ho ho! I organise Secret Santa in group chats.\n\n\
         Now that we've said hi, I can message you privately. Go back to your group and send /register to join. \
         When an admin sends /draw, I'll tell you here who you're buying a gift for.\n\n{}",
        Command::descriptions()
    )
}

fn group_title(msg: &Message) -> String {
    msg.chat.title().unwrap_or("your group").to_string()
}

async fn is_admin(bot: &Bot, msg: &Message) -> ResponseResult<bool> {
    let Some(user) = &msg.from else { return Ok(false) };
    let member = bot.get_chat_member(msg.chat.id, user.id).await?;
    Ok(member.is_privileged())
}

async fn reply(bot: &Bot, msg: &Message, text: &str) -> ResponseResult<()> {
    bot.send_message(msg.chat.id, text).parse_mode(ParseMode::Html).await?;
    Ok(())
}
