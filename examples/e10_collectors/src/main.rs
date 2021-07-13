//! This example will showcase the beauty of collectors.
//! They allow to await messages or reactions from a user in the middle
//! of a control flow, one being a command.
use std::{collections::HashSet, env, time::Duration};

use serenity::{
    async_trait,
    collector::{EventCollectorBuilder, MessageCollectorBuilder},
    framework::standard::{
        help_commands,
        macros::{command, group, help},
        Args,
        CommandGroup,
        CommandResult,
        HelpOptions,
        StandardFramework,
    },
    // Collectors are streams, that means we can use `StreamExt` and
    // `TryStreamExt`.
    futures::stream::StreamExt,
    http::Http,
    model::prelude::*,
    prelude::*,
};

#[group("collector")]
#[commands(challenge)]
struct Collector;

#[help]
async fn my_help(
    context: &Context,
    msg: &Message,
    args: Args,
    help_options: &'static HelpOptions,
    groups: &[&'static CommandGroup],
    owners: HashSet<UserId>,
) -> CommandResult {
    let _ = help_commands::with_embeds(context, msg, args, help_options, groups, owners).await;
    Ok(())
}

struct Handler;

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, _: Context, ready: Ready) {
        println!("{} is connected!", ready.user.name);
    }
}

#[tokio::main]
async fn main() {
    // Configure the client with your Discord bot token in the environment.
    let token = env::var("DISCORD_TOKEN").expect("Expected a token in the environment");

    let http = Http::new_with_token(&token);

    // We will fetch your bot's id.
    let bot_id = match http.get_current_application_info().await {
        Ok(info) => info.id,
        Err(why) => panic!("Could not access application info: {:?}", why),
    };

    let framework = StandardFramework::new()
        .configure(|c| {
            c.with_whitespace(true).on_mention(Some(bot_id)).prefix("~").delimiters(vec![", ", ","])
        })
        .help(&MY_HELP)
        .group(&COLLECTOR_GROUP);

    let mut client = Client::builder(&token)
        .event_handler(Handler)
        .framework(framework)
        .await
        .expect("Err creating client");

    if let Err(why) = client.start().await {
        println!("Client error: {:?}", why);
    }
}

#[command]
async fn challenge(ctx: &Context, msg: &Message, _: Args) -> CommandResult {
    let mut score = 0u32;
    let _ = msg.reply(ctx, "How was that crusty crab called again? 10 seconds time!").await;

    // There are methods implemented for some models to conveniently collect replies.
    // This one returns a future that will await a single message only.
    // The other method for messages is called `await_replies` and returns a future
    // which builds a stream to easily handle them.
    if let Some(answer) = &msg.author.await_reply(&ctx).timeout(Duration::from_secs(10)).await {
        if answer.content.to_lowercase() == "ferris" {
            let _ = answer.reply(ctx, "That's correct!").await;
            score += 1;
        } else {
            let _ = answer.reply(ctx, "Wrong, it's Ferris!").await;
        }
    } else {
        let _ = msg.reply(ctx, "No answer within 10 seconds.").await;
    };

    let react_msg = msg
        .reply(ctx, "React with the reaction representing 1, you got 10 seconds!")
        .await
        .unwrap();

    // The message model has a way to collect reactions on it.
    // Other methods are `await_n_reactions` and `await_all_reactions`.
    // Same goes for messages!
    if let Some(reaction) = &react_msg
        .await_reaction(&ctx)
        .timeout(Duration::from_secs(10))
        .author_id(msg.author.id)
        .await
    {
        // By default, the collector will collect only added reactions.
        // We could also pattern-match the reaction in case we want
        // to handle added or removed reactions.
        // In this case we will just get the inner reaction.
        let emoji = &reaction.as_inner_ref().emoji;

        let _ = match emoji.as_data().as_str() {
            "1️⃣" => {
                score += 1;
                msg.reply(ctx, "That's correct!").await
            },
            _ => msg.reply(ctx, "Wrong!").await,
        };
    } else {
        let _ = msg.reply(ctx, "No reaction within 10 seconds.").await;
    };

    let _ = msg.reply(ctx, "Write 5 messages in 10 seconds").await;

    // We can create a collector from scratch too using this builder future.
    let collector = MessageCollectorBuilder::new(&ctx)
    // Only collect messages by this user.
        .author_id(msg.author.id)
        .channel_id(msg.channel_id)
        .collect_limit(5u32)
        .timeout(Duration::from_secs(10))
    // Build the collector.
        .await;

    // Let's acquire borrow HTTP to send a message inside the `async move`.
    let http = &ctx.http;

    // We want to process each message and get the length.
    // There are a couple of ways to do this. Folding the stream with `fold`
    // is one way.
    // Using `then` to first reply and then create a new stream with all
    // messages is another way to do it, which can be nice if you want
    // to further process the messages.
    // If you don't want to collect the stream, `for_each` may be sufficient.
    let collected: Vec<_> = collector
        .then(|msg| async move {
            let _ = msg.reply(http, format!("I repeat: {}", msg.content)).await;

            msg
        })
        .collect()
        .await;

    if collected.len() >= 5 {
        score += 1;
    }

    // We can also collect arbitrary events using the generic EventCollector. For example, here we
    // collect updates to the messages that the user sent above and check for them updating all 5 of
    // them.
    let builder = EventCollectorBuilder::new(&ctx)
        .add_event_type(EventType::MessageUpdate)
        .timeout(Duration::from_secs(20));
    // Only collect MessageUpdate events for the 5 MessageIds we're interested in.
    let mut collector = collected.iter().fold(builder, |b, msg| b.add_message_id(msg.id)).await?;

    let _ = msg.reply(ctx, "Edit each of those 5 messages in 20 seconds").await;
    let mut edited = HashSet::new();
    while let Some(event) = collector.next().await {
        match event.as_ref() {
            Event::MessageUpdate(e) => {
                edited.insert(e.id);
            },
            e => panic!("Unexpected event type received: {:?}", e.event_type()),
        }
        if edited.len() >= 5 {
            break;
        }
    }

    if edited.len() >= 5 {
        score += 1;
        let _ = msg.reply(ctx, "Great! You edited 5 out of 5").await;
    } else {
        let _ = msg.reply(ctx, &format!("You only edited {} out of 5", edited.len())).await;
    }

    let _ = msg
        .reply(ctx, &format!("TIME'S UP! You completed {} out of 4 tasks correctly!", score))
        .await;

    Ok(())
}
