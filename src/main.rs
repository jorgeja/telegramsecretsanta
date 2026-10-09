mod santa;
mod store;

use std::sync::Arc;

use teloxide::{
    prelude::*,
    types::{InlineKeyboardButton, InlineKeyboardMarkup, Me, ParseMode, User},
    utils::{command::BotCommands, html},
};
use url::Url;

use store::{Game, Participant, Store};

/// Fewer than this and everyone can work out who their Santa is.
const MIN_PARTICIPANTS: usize = 3;

const NOT_STARTED: &str = "No Secret Santa is running in this group yet. A group admin can /start one.";

/// The join button opens a private chat with the bot via `t.me/<bot>?start=join_<chat id>`.
/// Pressing Start there sends `/start join_<chat id>`, which both lets the bot message the
/// person and joins them up, so nobody has to say hi to the bot separately.
const JOIN_PAYLOAD_PREFIX: &str = "join_";

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "🎅 Secret Santa bot commands:")]
enum Command {
    /// Show this help.
    Help,
    /// Start a Secret Santa in this group (admins only). You'll organise it and draw names.
    Start(String),
    /// Join this group's Secret Santa.
    Join,
    /// Leave this group's Secret Santa.
    Leave,
    /// Show who has registered.
    List,
    /// Draw names and DM everyone their match (organiser only).
    Draw,
    /// Cancel this group's Secret Santa so a new one can be started (admins only).
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
        Command::Start(payload) if !is_group => private_start(&bot, &msg, &store, &payload).await,
        Command::MySanta => my_santa(&bot, &msg, &store).await,
        _ if !is_group => reply(&bot, &msg, "This command only works in a group chat.").await,
        Command::Start(_) => start(&bot, &msg, &me, &store).await,
        Command::Join => join(&bot, &msg, &me, &store).await,
        Command::Leave => leave(&bot, &msg, &store).await,
        Command::List => list(&bot, &msg, &store).await,
        Command::Draw => draw(&bot, &msg, &store).await,
        Command::Reset => reset(&bot, &msg, &store).await,
    }
}

async fn start(bot: &Bot, msg: &Message, me: &Me, store: &Store) -> ResponseResult<()> {
    let Some(user) = &msg.from else { return Ok(()) };
    if !is_admin(bot, msg).await? {
        return reply(bot, msg, "Only group admins can start a Secret Santa.").await;
    }

    let organizer = Participant::from_user(user);
    let game = Game {
        title: group_title(msg),
        organizer: Some(organizer.clone()),
        participants: Vec::new(),
        assignments: None,
    };
    match store.start(msg.chat.id, game) {
        Ok(()) => {
            let text = format!(
                "🎅 {name} started a Secret Santa! Tap the button below to join. When everyone's in, {name} will /draw names.",
                name = organizer.display()
            );
            reply_with_join_button(bot, msg, me, &text).await
        }
        Err(existing) => {
            let text = format!(
                "A Secret Santa is already running here.{} An admin can /reset it to start over.",
                existing.map(|o| format!(" It's organised by {}.", o.display())).unwrap_or_default()
            );
            reply(bot, msg, &text).await
        }
    }
}

/// `/start` in a private chat: either someone tapped a group's join button, or they're just
/// saying hi.
async fn private_start(bot: &Bot, msg: &Message, store: &Store, payload: &str) -> ResponseResult<()> {
    let Some(user) = &msg.from else { return Ok(()) };
    let Some(chat) = payload.strip_prefix(JOIN_PAYLOAD_PREFIX).and_then(|id| id.parse().ok()).map(ChatId) else {
        return reply(bot, msg, &welcome_text()).await;
    };

    // Join links can be forwarded, so only let members of the group into its Secret Santa.
    let is_member = bot.get_chat_member(chat, user.id).await.is_ok_and(|m| m.kind.is_present());
    if !is_member {
        return reply(bot, msg, "You need to be a member of that group to join its Secret Santa.").await;
    }

    match add_participant(bot, store, chat, user).await? {
        Ok(title) => reply(bot, msg, &joined_text(&title)).await,
        Err(reason) => reply(bot, msg, reason).await,
    }
}

async fn join(bot: &Bot, msg: &Message, me: &Me, store: &Store) -> ResponseResult<()> {
    let Some(user) = &msg.from else { return Ok(()) };
    if let Some(reason) = store.read(msg.chat.id, |game| join_blocker(game, user.id)) {
        return reply(bot, msg, reason).await;
    }

    // Bots can only message people who have opened a private chat with them. If we can't,
    // send them through the join button instead, which opens that chat and joins in one go.
    let can_message = bot
        .send_message(user.id, joined_text(&group_title(msg)))
        .parse_mode(ParseMode::Html)
        .await
        .is_ok();
    if !can_message {
        let text = format!(
            "{}, tap the button below and press <b>Start</b> to join. That lets me send you your match privately.",
            Participant::from_user(user).display()
        );
        return reply_with_join_button(bot, msg, me, &text).await;
    }

    if let Err(reason) = add_participant(bot, store, msg.chat.id, user).await? {
        reply(bot, msg, reason).await?;
    }
    Ok(())
}

/// Why `user` can't join `game` right now, if anything.
fn join_blocker(game: Option<&Game>, user: UserId) -> Option<&'static str> {
    match game {
        None => Some(NOT_STARTED),
        Some(game) if game.is_drawn() => Some("Names have already been drawn for this round."),
        Some(game) if game.is_registered(user) => Some("You're already registered! 🎄"),
        Some(_) => None,
    }
}

/// Adds `user` to the Secret Santa in `chat` and announces it there. Returns the group's
/// title, or why they couldn't join.
async fn add_participant(
    bot: &Bot,
    store: &Store,
    chat: ChatId,
    user: &User,
) -> ResponseResult<Result<String, &'static str>> {
    let participant = Participant::from_user(user);
    let result = store
        .update(chat, |game| match join_blocker(Some(game), user.id) {
            Some(reason) => Err(reason),
            None => {
                game.participants.push(participant.clone());
                Ok((game.title.clone(), game.participants.len()))
            }
        })
        .unwrap_or(Err(NOT_STARTED));

    match result {
        Ok((title, count)) => {
            let text = format!("🎄 {} joined the Secret Santa! ({count} registered)", participant.display());
            send(bot, chat, &text).await?;
            Ok(Ok(title))
        }
        Err(reason) => Ok(Err(reason)),
    }
}

async fn leave(bot: &Bot, msg: &Message, store: &Store) -> ResponseResult<()> {
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
    match result.unwrap_or(Err(NOT_STARTED)) {
        Ok(count) => {
            let name = Participant::from_user(user).display();
            reply(bot, msg, &format!("👋 {name} left the Secret Santa. ({count} registered)")).await
        }
        Err(text) => reply(bot, msg, text).await,
    }
}

async fn list(bot: &Bot, msg: &Message, store: &Store) -> ResponseResult<()> {
    let text = store.read(msg.chat.id, |game| match game {
        None => NOT_STARTED.to_string(),
        Some(game) if game.participants.is_empty() => format!(
            "🎅 Nobody has joined the Secret Santa yet — send /join to take part!{}",
            organised_by(game)
        ),
        Some(game) => {
            let names: Vec<String> = game.participants.iter().map(|p| format!("• {}", p.display())).collect();
            let status = if game.is_drawn() {
                "Names have been drawn! 🎁"
            } else {
                "Registration is open — use /join to join in."
            };
            format!(
                "🎅 <b>Secret Santa participants ({})</b>{}\n\n{}\n\n{status}",
                names.len(),
                organised_by(game),
                names.join("\n")
            )
        }
    });
    reply(bot, msg, &text).await
}

fn organised_by(game: &Game) -> String {
    match &game.organizer {
        Some(organizer) => format!("\nOrganised by {}", organizer.display()),
        None => String::new(),
    }
}

async fn draw(bot: &Bot, msg: &Message, store: &Store) -> ResponseResult<()> {
    let Some(user) = &msg.from else { return Ok(()) };
    let Some(organizer) = store.read(msg.chat.id, |game| game.map(|g| g.organizer.clone())) else {
        return reply(bot, msg, NOT_STARTED).await;
    };
    if let Some(organizer) = organizer
        && organizer.id != user.id
    {
        let text = format!(
            "Only {}, who started this Secret Santa, can draw names.",
            organizer.display()
        );
        return reply(bot, msg, &text).await;
    }
    if !is_admin(bot, msg).await? {
        return reply(bot, msg, "You need to be a group admin to draw names.").await;
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
    let (title, pairs) = match result.unwrap_or_else(|| Err(NOT_STARTED.to_string())) {
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
    reply(bot, msg, "🔄 Secret Santa has been reset. A group admin can /start a new one.").await
}

async fn my_santa(bot: &Bot, msg: &Message, store: &Store) -> ResponseResult<()> {
    let Some(user) = &msg.from else { return Ok(()) };
    let matches = store.matches_for(user.id);

    let text = if matches.is_empty() {
        "You don't have a Secret Santa match yet. Send /join in your group and wait for the draw!"
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

fn joined_text(group_title: &str) -> String {
    format!(
        "👋 You're in the Secret Santa for <b>{}</b>! I'll send your match here once names are drawn.",
        html::escape(group_title)
    )
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
         To take part, tap the join button in your group or send /join there. \
         When the organiser draws names, I'll tell you here who you're buying a gift for.\n\n{}",
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
    send(bot, msg.chat.id, text).await
}

/// Replies with a button that opens a private chat with the bot and joins this group's
/// Secret Santa (see [`JOIN_PAYLOAD_PREFIX`]).
async fn reply_with_join_button(bot: &Bot, msg: &Message, me: &Me, text: &str) -> ResponseResult<()> {
    let link = format!("https://t.me/{}?start={JOIN_PAYLOAD_PREFIX}{}", me.username(), msg.chat.id);
    let url = Url::parse(&link).expect("bot usernames and chat ids are valid in a URL");
    let button = InlineKeyboardMarkup::new([[InlineKeyboardButton::url("🎄 Join Secret Santa", url)]]);
    bot.send_message(msg.chat.id, text).parse_mode(ParseMode::Html).reply_markup(button).await?;
    Ok(())
}

async fn send(bot: &Bot, chat: ChatId, text: &str) -> ResponseResult<()> {
    bot.send_message(chat, text).parse_mode(ParseMode::Html).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_parses_with_and_without_a_join_payload() {
        let plain = Command::parse("/start", "santa_bot");
        assert!(matches!(plain, Ok(Command::Start(p)) if p.is_empty()));

        let join = Command::parse("/start join_-1001234567890", "santa_bot");
        assert!(matches!(join, Ok(Command::Start(p)) if p == "join_-1001234567890"));
    }
}
