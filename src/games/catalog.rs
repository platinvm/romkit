use serde::{Serialize, Serializer};

use crate::platforms::{Identity, Platform};

/// Declares [`GameId`] from one row per game, so that its slug, name, and header signature
/// cannot drift apart. A signature is the header title on Game Boy platforms and the first
/// three characters of the product code elsewhere.
macro_rules! catalog {
    ($( $id:ident: $slug:literal, $name:literal, $platform:ident $signature:literal; )+) => {
        /// Every game romkit recognizes, whether or not it can extract it yet.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum GameId {
            $( $id, )+
        }

        impl GameId {
            pub const ALL: &[Self] = &[$( Self::$id, )+];

            /// A stable kebab-case identifier, e.g. `pokemon-platinum`.
            pub const fn slug(self) -> &'static str {
                match self {
                    $( Self::$id => $slug, )+
                }
            }

            /// The display name, e.g. `Pokémon Platinum`.
            pub const fn name(self) -> &'static str {
                match self {
                    $( Self::$id => $name, )+
                }
            }

            const fn signature(self) -> (Platform, &'static str) {
                match self {
                    $( Self::$id => (Platform::$platform, $signature), )+
                }
            }
        }
    };
}

catalog! {
    PokemonRed: "pokemon-red", "Pokémon Red", GameBoy "POKEMON RED";
    PokemonGreen: "pokemon-green", "Pokémon Green", GameBoy "POKEMON GREEN";
    PokemonBlue: "pokemon-blue", "Pokémon Blue", GameBoy "POKEMON BLUE";
    PokemonYellow: "pokemon-yellow", "Pokémon Yellow", GameBoyColor "POKEMON YELLOW";
    PokemonGold: "pokemon-gold", "Pokémon Gold", GameBoyColor "POKEMON_GLD";
    PokemonSilver: "pokemon-silver", "Pokémon Silver", GameBoyColor "POKEMON_SLV";
    PokemonCrystal: "pokemon-crystal", "Pokémon Crystal", GameBoyColor "PM_CRYSTAL";
    PokemonRuby: "pokemon-ruby", "Pokémon Ruby", GameBoyAdvance "AXV";
    PokemonSapphire: "pokemon-sapphire", "Pokémon Sapphire", GameBoyAdvance "AXP";
    PokemonEmerald: "pokemon-emerald", "Pokémon Emerald", GameBoyAdvance "BPE";
    PokemonFireRed: "pokemon-firered", "Pokémon FireRed", GameBoyAdvance "BPR";
    PokemonLeafGreen: "pokemon-leafgreen", "Pokémon LeafGreen", GameBoyAdvance "BPG";
    PokemonDiamond: "pokemon-diamond", "Pokémon Diamond", NintendoDs "ADA";
    PokemonPearl: "pokemon-pearl", "Pokémon Pearl", NintendoDs "APA";
    PokemonPlatinum: "pokemon-platinum", "Pokémon Platinum", NintendoDs "CPU";
    PokemonHeartGold: "pokemon-heartgold", "Pokémon HeartGold", NintendoDs "IPK";
    PokemonSoulSilver: "pokemon-soulsilver", "Pokémon SoulSilver", NintendoDs "IPG";
    PokemonBlack: "pokemon-black", "Pokémon Black", NintendoDs "IRB";
    PokemonWhite: "pokemon-white", "Pokémon White", NintendoDs "IRA";
    PokemonBlack2: "pokemon-black-2", "Pokémon Black 2", NintendoDs "IRE";
    PokemonWhite2: "pokemon-white-2", "Pokémon White 2", NintendoDs "IRD";
    PokemonX: "pokemon-x", "Pokémon X", Nintendo3ds "EKJ";
    PokemonY: "pokemon-y", "Pokémon Y", Nintendo3ds "EK2";
    PokemonOmegaRuby: "pokemon-omega-ruby", "Pokémon Omega Ruby", Nintendo3ds "ECR";
    PokemonAlphaSapphire: "pokemon-alpha-sapphire", "Pokémon Alpha Sapphire", Nintendo3ds "ECL";
    PokemonSun: "pokemon-sun", "Pokémon Sun", Nintendo3ds "BND";
    PokemonMoon: "pokemon-moon", "Pokémon Moon", Nintendo3ds "BNE";
    PokemonUltraSun: "pokemon-ultra-sun", "Pokémon Ultra Sun", Nintendo3ds "A2A";
    PokemonUltraMoon: "pokemon-ultra-moon", "Pokémon Ultra Moon", Nintendo3ds "A2B";
}

impl GameId {
    /// The game a ROM holds, from its header alone.
    pub fn identify(identity: &Identity) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|game| game.matches(identity))
    }

    pub fn matches(self, identity: &Identity) -> bool {
        let game_boy = |platform| matches!(platform, Platform::GameBoy | Platform::GameBoyColor);
        match self.signature() {
            // Color-enhanced cartridges of a game share its title, so either Game Boy matches.
            (platform, title) if game_boy(platform) => {
                game_boy(identity.platform) && identity.title == title
            }
            (platform, code) => {
                identity.platform == platform
                    && identity
                        .code
                        .is_some_and(|game| game.title() == code.as_bytes())
            }
        }
    }
}

/// Serialized as its slug.
impl Serialize for GameId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.slug())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::platforms::{GameCode, Region};

    #[test]
    fn slugs_are_unique_kebab_case() {
        let slugs: HashSet<_> = GameId::ALL.iter().map(|game| game.slug()).collect();
        assert_eq!(slugs.len(), GameId::ALL.len());
        for slug in slugs {
            assert!(
                slug.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            );
        }
    }

    #[test]
    fn matches_codes_in_any_region_but_only_on_their_platform() {
        let mut platinum = Identity {
            platform: Platform::NintendoDs,
            title: "POKEMON PL".into(),
            code: Some(GameCode(*b"CPUJ")),
            region: Region::Japan,
            revision: 0,
        };
        assert_eq!(GameId::identify(&platinum), Some(GameId::PokemonPlatinum));
        platinum.platform = Platform::GameBoyAdvance;
        assert_eq!(GameId::identify(&platinum), None);
    }
}
