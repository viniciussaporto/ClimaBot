//! The competitions the bot follows. Keys are SportScore's competition slugs,
//! checked against its standings endpoint on 2026-11-18; `sportscore_name`
//! is how its fixture lists spell the competition. `football_data` is the
//! football-data.org code (free tier) used as a delayed fallback.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// One league table.
    League,
    /// Group tables, then (usually) knockouts.
    Groups,
    /// Knockout only: no meaningful table.
    Cup,
}

#[derive(Debug)]
pub struct Competition {
    pub key: &'static str,
    pub name: &'static str,
    pub sportscore_name: &'static str,
    pub region: &'static str,
    /// Country of the clubs, for picking the right team on API-Football.
    /// Empty for continental competitions.
    pub country: &'static str,
    pub kind: Kind,
    pub football_data: Option<&'static str>,
    /// Followed closely by default: match details are polled while live.
    pub priority: bool,
}

macro_rules! comp {
    ($key:literal, $name:literal, $ss:literal, $region:literal, $country:literal, $kind:ident, $fd:expr, $prio:literal) => {
        Competition {
            key: $key,
            name: $name,
            sportscore_name: $ss,
            region: $region,
            country: $country,
            kind: Kind::$kind,
            football_data: $fd,
            priority: $prio,
        }
    };
}

pub static COMPETITIONS: &[Competition] = &[
    // Brazil
    comp!("brazilian-serie-a", "Brasileirão Série A", "Brazilian Serie A", "🇧🇷 Brazil", "Brazil", League, Some("BSA"), true),
    comp!("brazilian-serie-b", "Brasileirão Série B", "Brazilian Serie B", "🇧🇷 Brazil", "Brazil", League, None, true),
    comp!("brazilian-serie-c", "Brasileirão Série C", "Brazilian Serie C", "🇧🇷 Brazil", "Brazil", Groups, None, false),
    comp!("brazilian-cup", "Copa do Brasil", "Brazilian Cup", "🇧🇷 Brazil", "Brazil", Cup, None, true),
    comp!("supercopa-do-brasil", "Supercopa do Brasil", "Supercopa do Brasil", "🇧🇷 Brazil", "Brazil", Cup, None, true),
    comp!("brazilian-copa-do-nordeste", "Copa do Nordeste", "Brazilian Copa do Nordeste", "🇧🇷 Brazil", "Brazil", Groups, None, false),
    comp!("brazilian-copa-verde", "Copa Verde", "Brazilian Copa Verde", "🇧🇷 Brazil", "Brazil", Groups, None, false),
    comp!("brazilian-campeonato-paulista-a1", "Paulistão", "Brazilian Campeonato Paulista A1", "🇧🇷 Brazil", "Brazil", League, None, true),
    comp!("brazilian-campeonato-carioca-a", "Campeonato Carioca", "Brazilian Campeonato Carioca A", "🇧🇷 Brazil", "Brazil", Groups, None, true),
    comp!("brazilian-campeonato-mineiro-division-1", "Campeonato Mineiro", "Brazilian Campeonato Mineiro Division 1", "🇧🇷 Brazil", "Brazil", Groups, None, false),
    comp!("brazilian-campeonato-gaucho", "Campeonato Gaúcho", "Brazilian Campeonato Gaucho", "🇧🇷 Brazil", "Brazil", Groups, None, false),
    // South America
    comp!("conmebol-copa-libertadores", "Copa Libertadores", "CONMEBOL Copa Libertadores", "🌎 South America", "", Groups, Some("CLI"), true),
    comp!("conmebol-copa-sudamericana", "Copa Sudamericana", "CONMEBOL Copa Sudamericana", "🌎 South America", "", Groups, None, true),
    // Europe
    comp!("uefa-champions-league", "UEFA Champions League", "UEFA Champions League", "🇪🇺 Europe", "", League, Some("CL"), false),
    comp!("uefa-europa-league", "UEFA Europa League", "UEFA Europa League", "🇪🇺 Europe", "", League, None, false),
    comp!("uefa-europa-conference-league", "UEFA Conference League", "UEFA Europa Conference League", "🇪🇺 Europe", "", League, None, false),
    comp!("english-premier-league", "Premier League", "English Premier League", "🏴 England", "England", League, Some("PL"), false),
    comp!("fa-cup", "FA Cup", "FA Cup", "🏴 England", "England", Cup, None, false),
    comp!("english-football-league-cup", "EFL Cup", "English Football League Cup", "🏴 England", "England", Cup, None, false),
    comp!("spanish-la-liga", "La Liga", "Spanish La Liga", "🇪🇸 Spain", "Spain", League, Some("PD"), false),
    comp!("copa-del-rey", "Copa del Rey", "Copa del Rey", "🇪🇸 Spain", "Spain", Cup, None, false),
    comp!("bundesliga", "Bundesliga", "Bundesliga", "🇩🇪 Germany", "Germany", League, Some("BL1"), false),
    comp!("dfb-pokal", "DFB-Pokal", "DFB Pokal", "🇩🇪 Germany", "Germany", Cup, None, false),
    comp!("italian-serie-a", "Serie A (Italy)", "Italian Serie A", "🇮🇹 Italy", "Italy", League, Some("SA"), false),
    comp!("coppa-italia", "Coppa Italia", "Coppa Italia", "🇮🇹 Italy", "Italy", Cup, None, false),
    comp!("french-ligue-1", "Ligue 1", "French Ligue 1", "🇫🇷 France", "France", League, Some("FL1"), false),
    comp!("coupe-de-france", "Coupe de France", "Coupe de France", "🇫🇷 France", "France", Cup, None, false),
    comp!("portuguese-primera-liga", "Primeira Liga", "Portuguese Primera Liga", "🇵🇹 Portugal", "Portugal", League, Some("PPL"), false),
    comp!("portuguese-cup", "Taça de Portugal", "Portuguese Cup", "🇵🇹 Portugal", "Portugal", Cup, None, false),
    comp!("portuguese-league-cup", "Taça da Liga", "Portuguese League Cup", "🇵🇹 Portugal", "Portugal", Cup, None, false),
];

pub fn by_key(key: &str) -> Option<&'static Competition> {
    COMPETITIONS.iter().find(|c| c.key == key)
}

/// The tracked competition a SportScore fixture belongs to, by its spelling of the name.
pub fn by_sportscore_name(name: &str) -> Option<&'static Competition> {
    COMPETITIONS.iter().find(|c| c.sportscore_name.eq_ignore_ascii_case(name))
}

/// Find a competition from what a user typed: a key, or part of a name.
pub fn find(input: &str) -> Option<&'static Competition> {
    let q = crate::football::fold(input);
    by_key(input.trim())
        .or_else(|| COMPETITIONS.iter().find(|c| crate::football::fold(c.name) == q))
        .or_else(|| COMPETITIONS.iter().find(|c| crate::football::fold(c.name).contains(&q)))
        .or_else(|| COMPETITIONS.iter().find(|c| crate::football::fold(c.sportscore_name).contains(&q)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_names_are_unique() {
        for (i, a) in COMPETITIONS.iter().enumerate() {
            for b in &COMPETITIONS[i + 1..] {
                assert_ne!(a.key, b.key);
                assert_ne!(a.sportscore_name, b.sportscore_name);
                assert_ne!(a.name, b.name);
            }
            // Discord choice names and values are limited to 100 characters.
            assert!(a.key.len() <= 100 && a.name.len() <= 80);
        }
    }

    #[test]
    fn lookups() {
        assert_eq!(by_sportscore_name("Brazilian Cup").unwrap().name, "Copa do Brasil");
        assert_eq!(find("copa do brasil").unwrap().key, "brazilian-cup");
        assert_eq!(find("libertadores").unwrap().key, "conmebol-copa-libertadores");
        assert_eq!(find("Gaucho").unwrap().key, "brazilian-campeonato-gaucho");
        assert_eq!(find("premier").unwrap().key, "english-premier-league");
        assert!(find("narnia league").is_none());
    }
}
