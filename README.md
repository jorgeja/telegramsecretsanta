# Secret Santa Telegram bot

A Telegram bot that runs Secret Santa in a group chat. People register in the group, an admin
draws names, and the bot privately tells each person who they're buying a gift for.

## How it works

1. Add the bot to your group.
2. Everyone opens a private chat with the bot and presses **Start**. Telegram only lets bots
   message people who have done this.
3. Everyone sends `/register` in the group. The bot checks it can message them privately and
   tells them if they still need to press Start.
4. A group admin sends `/draw`. Each participant gets a private message with their match.

Everyone is linked into one big circle, so nobody gets themselves and you never get two people
who just have each other.

## Commands

| Command | Where | Description |
| --- | --- | --- |
| `/register` (`/join`) | group | Join this group's Secret Santa |
| `/unregister` (`/leave`) | group | Leave (only before the draw) |
| `/list` | group | Show who has registered |
| `/draw` | group, admins | Draw names and DM everyone their match |
| `/reset` | group, admins | Clear the round and start over |
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
