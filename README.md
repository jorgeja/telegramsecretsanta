# Secret Santa Telegram bot

A Telegram bot that runs Secret Santa in a group chat. An admin starts a round, people join in
the group, the organiser draws names, and the bot privately tells each person who they're buying
a gift for.

## How it works

1. Add the bot to your group.
2. A group admin sends `/start` in the group. They become the organiser and are automatically
   the first participant.
3. Everyone taps the **Join Secret Santa** button under the bot's message. It opens a private
   chat with the bot, and pressing **Start** there joins them. Telegram only lets bots message
   people who have started a chat with them, so this one tap covers both.
   People who have already chatted with the bot can just send `/join` in the group.
4. The organiser sends `/draw`. Each participant gets a private message with their match.

Everyone is linked into one big circle, so nobody gets themselves and you never get two people
who just have each other.

## Commands

| Command | Where | Description |
| --- | --- | --- |
| `/start` | group, admins | Start a Secret Santa; you become its organiser |
| `/start` | private chat | Sent by the Join button; joins you to that group's Secret Santa |
| `/join` | group | Join this group's Secret Santa |
| `/leave` | group | Leave (only before the draw) |
| `/list` | group | Show who has registered |
| `/draw` | group, organiser | Draw names and DM everyone their match. The organiser must still be an admin. |
| `/reset` | group, admins | Cancel the round so a new one can be started |
| `/mysanta` | anywhere | Get your match sent to you privately again |
| `/help` | anywhere | Show the commands |

## Setup

Create a bot with [@BotFather](https://t.me/BotFather) and copy its token.

### Docker

```sh
cp .env.example .env   # then put your token in .env
docker compose up -d --build
```

Registrations and matches are saved in the `santa-data` volume, so they survive restarts.

### Running locally

```sh
TELOXIDE_TOKEN=<your token> RUST_LOG=info cargo run --release
```

State is saved to `santa_state.json` in the current directory. Set `SANTA_STATE_FILE` to
change the location.

> The state file contains everyone's matches, so whoever runs the bot can see who has who.
