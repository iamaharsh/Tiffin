//! The data sources Tiffin is built on, with the credit and licence wording each one asks for.
//! Shown on the About screen; the wording follows each source's own terms page (checked 2026-11-21).

use serde::Serialize;

/// One credited data source.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Source {
    pub name: &'static str,
    /// What Tiffin uses it for.
    pub used_for: &'static str,
    /// The licence or terms, in words.
    pub licence: &'static str,
    /// The attribution or citation, as the source asks for it.
    pub credit: &'static str,
    pub link: &'static str,
}

/// Every source this build holds data from. The Compendium is listed only when its rows are compiled in
/// (public builds carry a stand-in until its maintainers reply, see `tracker::exercise`).
pub fn data_sources() -> Vec<Source> {
    sources(crate::tracker::exercise::full_table())
}

fn sources(compendium: bool) -> Vec<Source> {
    let mut out = vec![
        Source {
            name: "USDA FoodData Central",
            used_for: "Foods stored on this PC: Foundation Foods (2026-04-30), SR Legacy (2018-04) and FNDDS 2021-2023.",
            licence: "Public domain, published under CC0 1.0 Universal.",
            credit: "U.S. Department of Agriculture, Agricultural Research Service, Beltsville Human Nutrition Research Center. FoodData Central. [Internet]. Available from https://fdc.nal.usda.gov/.",
            link: "https://fdc.nal.usda.gov/",
        },
        Source {
            name: "Health Canada, Canadian Nutrient File",
            used_for: "Foods stored on this PC: the 2026 edition.",
            licence: "Open Government Licence – Canada.",
            credit: "Contains information licensed under the Open Government Licence – Canada.",
            link: "https://open.canada.ca/en/open-government-licence-canada",
        },
        Source {
            name: "Open Food Facts",
            used_for: "Search Online and barcode lookups. Tiffin sends the search text or the barcode, with the contact email Open Food Facts asks apps to include.",
            licence: "Database under the Open Database Licence (ODbL); its contents under the Database Contents Licence.",
            credit: "Contains information from Open Food Facts, available under the Open Database Licence.",
            link: "https://world.openfoodfacts.org/",
        },
    ];
    if compendium {
        out.push(Source {
            name: "2024 Adult Compendium of Physical Activities",
            used_for: "Activities and their MET values for logged exercise. Values are unaltered and activities are not combined.",
            licence: "Free to use, including commercially, on the condition that MET values are not changed and the Compendium is cited.",
            credit: "Herrmann SD, Willis EA, Ainsworth BE, et al. 2024 Adult Compendium of Physical Activities: A third update of the energy costs of human activities. J Sport Health Sci 2024;13(1):6-12.",
            link: "https://pacompendium.com/",
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_source_is_credited_as_it_asks() {
        let s = sources(true);
        assert!(s.iter().all(|x| !x.licence.is_empty() && !x.credit.is_empty() && x.link.starts_with("https://")));
        let credit = |name: &str| s.iter().find(|x| x.name.starts_with(name)).map(|x| x.credit).unwrap_or_else(|| panic!("{name} is credited"));
        assert!(credit("USDA").contains("FoodData Central"));
        assert_eq!(credit("Health Canada"), "Contains information licensed under the Open Government Licence – Canada.");
        assert!(credit("Open Food Facts").contains("Open Database Licence"));
        assert!(credit("2024 Adult Compendium").starts_with("Herrmann SD"));
    }

    #[test]
    fn the_compendium_is_credited_only_when_its_rows_are_compiled_in() {
        assert!(!sources(false).iter().any(|x| x.name.contains("Compendium")));
        assert_eq!(sources(false).len() + 1, sources(true).len());
        assert_eq!(data_sources(), sources(crate::tracker::exercise::full_table()));
    }
}
