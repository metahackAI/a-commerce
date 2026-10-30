//! Product pictures for the demo store. The web draws each garment flat in its color
//! (`web/src/app/media/garment`), so a picture is a URL naming a shape and a color:
//! `/media/garment/wind-slate.svg`.

/// Garment shapes by the words a product name ends with, most specific first.
const SHAPES: &[(&str, &[&str])] = &[
    (
        "wind",
        &[
            "windbreaker",
            "anorak",
            "cagoule",
            "rain jacket",
            "shell jacket",
        ],
    ),
    ("parka", &["parka", "puffer"]),
    (
        "chore",
        &[
            "chore coat",
            "overcoat",
            "peacoat",
            "coat",
            "trench",
            "duffle",
        ],
    ),
    ("vest", &["gilet", "vest"]),
    (
        "field",
        &["jacket", "bomber", "harrington", "shacket", "trucker"],
    ),
    (
        "knit",
        &[
            "hoodie",
            "sweatshirt",
            "sweater",
            "cardigan",
            "crewneck",
            "turtleneck",
            "knit",
            "quarter-zip",
            "half-zip",
            "fleece",
            "pullover",
        ],
    ),
    ("shirt", &["overshirt", "shirt", "button-down", "blouse"]),
    ("dress", &["dress", "robe", "one-piece", "jumpsuit"]),
    ("skirt", &["skirt"]),
    ("shorts", &["shorts", "trunks", "boxers", "briefs"]),
    (
        "pants",
        &[
            "jeans", "pants", "trousers", "chinos", "joggers", "leggings", "tights",
        ],
    ),
    (
        "tee",
        &[
            "tee",
            "t-shirt",
            "top",
            "henley",
            "undershirt",
            "tank",
            "guard",
            "bra",
            "bralette",
            "set",
        ],
    ),
    ("boot", &["boots", "boot"]),
    (
        "shoe",
        &[
            "runners",
            "sneakers",
            "shoes",
            "brogues",
            "sandals",
            "high-tops",
            "low-tops",
            "clogs",
            "slides",
            "flip-flops",
            "loafers",
            "trainers",
        ],
    ),
    ("socks", &["socks"]),
    ("belt", &["belt"]),
    (
        "bag",
        &[
            "bag",
            "tote",
            "duffel",
            "backpack",
            "wallet",
            "case",
            "weekender",
            "pouch",
        ],
    ),
    ("hat", &["cap", "hat", "beanie"]),
    (
        "scarf",
        &[
            "scarf", "stole", "snood", "bandana", "wrap", "neck", "gloves",
        ],
    ),
];

/// The shape a product name draws as; `fold` when no word matches.
pub fn shape(name: &str) -> &'static str {
    let name = name.to_lowercase();
    SHAPES
        .iter()
        .find(|(_, words)| {
            words.iter().any(|w| {
                name == *w || name.ends_with(&format!(" {w}")) || name.contains(&format!(" {w} "))
            })
        })
        .map_or("fold", |(s, _)| s)
}

/// The picture URL of a product (or one variant) of this name in this color.
pub fn garment_image(name: &str, color: &str) -> String {
    let color = color.trim().to_lowercase().replace(' ', "-");
    format!("/media/garment/{}-{color}.svg", shape(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_pick_their_shapes() {
        assert_eq!(shape("Packable windbreaker"), "wind");
        assert_eq!(shape("Linen blend field jacket"), "field");
        assert_eq!(shape("Seersucker overshirt"), "shirt");
        assert_eq!(shape("Chore coat"), "chore");
        assert_eq!(shape("Merino fisherman sweater"), "knit");
        assert_eq!(shape("Terry trail shorts"), "shorts");
        assert_eq!(shape("Leather belt"), "belt");
        assert_eq!(shape("Umbrella"), "fold");
        assert_eq!(
            garment_image("Packable windbreaker", "Heather grey"),
            "/media/garment/wind-heather-grey.svg"
        );
    }
}
