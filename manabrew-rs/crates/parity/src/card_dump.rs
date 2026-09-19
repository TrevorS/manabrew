//! `parity card <name>...`: what the engine builds from a card script, face by face.

use manabrew_engine::card::Card;
use manabrew_engine::ids::PlayerId;

use crate::runner::LoadedData;

pub fn run_cli(args: &[String], data: &LoadedData) -> i32 {
    let names: Vec<&String> = args.iter().skip(1).collect();
    if names.is_empty() {
        eprintln!("usage: parity card <card name or token script>...");
        return 2;
    }
    let mut code = 0;
    for name in names {
        if let Some(rules) = data.db.get_by_card_name(name) {
            print_card(&Card::from_rules(rules, PlayerId(0)));
        } else if let Some((_, template)) = data
            .token_templates
            .iter()
            .find(|(script, _)| script.eq_ignore_ascii_case(name))
        {
            print_card(template);
        } else {
            eprintln!("card: {name}: neither a card nor a token script");
            code = 1;
        }
    }
    code
}

fn print_card(card: &Card) {
    println!(
        "== {} {}  {}",
        card.card_name, card.type_line, card.mana_cost
    );
    let keywords: Vec<String> = card.keywords.iter_strings().map(str::to_string).collect();
    if !keywords.is_empty() {
        println!("keywords: {}", keywords.join(" | "));
    }
    for ab in &card.activated_abilities {
        println!(
            "ability {} {}{:?} zone={:?}: {}",
            ab.ability_index,
            if ab.is_mana_ability { "mana " } else { "" },
            ab.ability_api,
            ab.activation_zone,
            ab.ability_text
        );
    }
    for trigger in &card.triggers {
        println!(
            "trigger {} {:?} execute={}: {}",
            trigger.id, trigger.kind, trigger.execute, trigger.description
        );
    }
    for st in &card.static_abilities {
        println!(
            "static {:?}: {}",
            st.modes,
            st.ir.description_text.as_deref().unwrap_or("")
        );
    }
    for re in &card.replacement_effects {
        println!(
            "replacement {:?}: {}",
            re.event,
            re.ir.description_text.as_deref().unwrap_or("")
        );
    }
    println!(
        "svars: {}",
        card.svars.keys().cloned().collect::<Vec<_>>().join(", ")
    );
    if let Some(other) = &card.other_part {
        println!(
            "-- other part {} [{:?}] {}{}",
            other.name,
            other.state_name,
            other.type_line,
            if other.is_modal { " (modal)" } else { "" }
        );
        let keywords: Vec<String> = other.keywords.iter_strings().map(str::to_string).collect();
        if !keywords.is_empty() {
            println!("keywords: {}", keywords.join(" | "));
        }
        for text in &other.abilities {
            println!("ability: {text}");
        }
        for trigger in &other.triggers {
            println!("trigger {:?}: {}", trigger.kind, trigger.description);
        }
        for st in &other.static_abilities {
            println!(
                "static {:?}: {}",
                st.modes,
                st.ir.description_text.as_deref().unwrap_or("")
            );
        }
    }
}
