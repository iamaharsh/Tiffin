//! Import rules for popular recipe sites. Each rule carries a small test page ("fixture") with the
//! site's layout; the rule is tested against it (cargo test and Help > Run self-tests).
//! When a site changes its layout, a rules update file can replace a rule by domain, and a rule
//! that stops finding anything on a live page is marked "needs update".
//!
//! Selectors are our own reading of each site's layout as of the "verified" date (estimated).

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, Debug, Default, PartialEq)]
#[serde(default)]
pub struct SiteRule {
    pub domain: String,
    pub name: String,
    pub version: u32,
    pub verified: String,
    pub title: Vec<String>,
    pub ingredients: Vec<String>,
    pub steps: Vec<String>,
    pub servings: Vec<String>,
    pub fixture: String,
    pub expect_ingredients: usize,
    pub expect_steps: usize,
}

fn v(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|s| s.to_string()).collect()
}

fn rule(
    domain: &str,
    name: &str,
    version: u32,
    title: &[&str],
    ingredients: &[&str],
    steps: &[&str],
    servings: &[&str],
    fixture: &str,
    ei: usize,
    es: usize,
) -> SiteRule {
    SiteRule {
        domain: domain.into(),
        name: name.into(),
        version,
        verified: "2026-10".into(),
        title: v(title),
        ingredients: v(ingredients),
        steps: v(steps),
        servings: v(servings),
        fixture: fixture.into(),
        expect_ingredients: ei,
        expect_steps: es,
    }
}

/// Rules checked against the live sites (others are checked against their sample pages only).
pub const LIVE_VERIFIED: &[(&str, &str)] =
    &[("allrecipes.com", "2026-11-03"), ("bbcgoodfood.com", "2026-11-03"), ("delish.com", "2026-11-03"), ("tasty.co", "2026-11-03")];

pub fn builtin_rules() -> Vec<SiteRule> {
    let mut v = builtin_rules_raw();
    for r in v.iter_mut() {
        if let Some((_, d)) = LIVE_VERIFIED.iter().find(|(dom, _)| *dom == r.domain) {
            r.verified = format!("{} (live site)", d);
        }
    }
    v
}

fn builtin_rules_raw() -> Vec<SiteRule> {
    vec![
        rule("allrecipes.com", "Allrecipes", 5, &["h1.article-heading", "h1"],
            &[".mm-recipes-structured-ingredients__list-item", ".mntl-structured-ingredients__list-item"],
            // v5 (verified against the live site 2026-11-03): only each step's own paragraph, not photo captions.
            &[".mm-recipes-steps__content li > p.mntl-sc-block-html", "ol.mntl-sc-block-group--OL > li > p"],
            &[],
            r#"<html><body><h1 class="article-heading">Easy Pancakes</h1>
<ul><li class="mm-recipes-structured-ingredients__list-item"><p><span data-ingredient-quantity="true">1 1/2</span> <span data-ingredient-unit="true">cups</span> <span data-ingredient-name="true">all-purpose flour</span></p></li>
<li class="mm-recipes-structured-ingredients__list-item"><p>3 1/2 teaspoons baking powder</p></li>
<li class="mm-recipes-structured-ingredients__list-item"><p>1 tablespoon white sugar</p></li>
<li class="mm-recipes-structured-ingredients__list-item"><p>1 1/4 cups milk</p></li></ul>
<div class="mm-recipes-steps__content"><ol class="mntl-sc-block-group--OL"><li class="mntl-sc-block-group--LI"><p class="mntl-sc-block-html">Sift the flour, baking powder and sugar together.</p><figure><figcaption><p>Dotdash Meredith Food Studios</p></figcaption></figure></li>
<li class="mntl-sc-block-group--LI"><p class="mntl-sc-block-html">Whisk in the milk until smooth.</p></li><li class="mntl-sc-block-group--LI"><p class="mntl-sc-block-html">Cook on a hot griddle for 2 minutes per side.</p></li></ol></div></body></html>"#, 4, 3),
        rule("foodnetwork.com", "Food Network", 3, &["h1.o-AssetTitle__a-Headline", "h1"],
            &[".o-Ingredients__a-Ingredient--CheckboxLabel", ".o-Ingredients__a-Ingredient"],
            &["li.o-Method__m-Step", ".o-Method__m-Body ol li"], &[".o-RecipeInfo__a-Description"],
            r#"<html><body><h1 class="o-AssetTitle__a-Headline"><span>Roast Chicken</span></h1>
<p class="o-Ingredients__a-Ingredient"><span class="o-Ingredients__a-Ingredient--CheckboxLabel">1 whole chicken</span></p>
<p class="o-Ingredients__a-Ingredient"><span class="o-Ingredients__a-Ingredient--CheckboxLabel">2 tablespoons butter</span></p>
<p class="o-Ingredients__a-Ingredient"><span class="o-Ingredients__a-Ingredient--CheckboxLabel">1 lemon, halved</span></p>
<ol><li class="o-Method__m-Step">Heat the oven to 425 degrees F.</li><li class="o-Method__m-Step">Roast the chicken for 1 hour 15 minutes.</li></ol></body></html>"#, 3, 2),
        // v4 (verified against the live site 2026-11-03): ingredients-list / method-steps layout.
        rule("bbcgoodfood.com", "BBC Good Food", 4, &["h1.heading-1", "h1"],
            &["li.ingredients-list__item", ".recipe__ingredients li"],
            &["li.method-steps__list-item .editor-content", ".recipe__method-steps li .editor-content"], &[],
            r#"<html><body><h1 class="heading-1">Easy Chocolate Cake</h1>
<ul class="ingredients-list list"><li class="ingredients-list__item list-item"><span class="ingredients-list__item-quantity">175g </span><a href="/glossary/butter"><span class="ingredients-list__item-ingredient">butter</span></a>, softened</li>
<li class="ingredients-list__item list-item"><span class="ingredients-list__item-quantity">175g </span><span class="ingredients-list__item-ingredient">caster sugar</span></li>
<li class="ingredients-list__item list-item"><span class="ingredients-list__item-quantity">3 </span><span class="ingredients-list__item-ingredient">large eggs</span></li>
<li class="ingredients-list__item list-item"><span class="ingredients-list__item-quantity">140g </span><span class="ingredients-list__item-ingredient">self-raising flour</span></li>
<li class="ingredients-list__item list-item"><span class="ingredients-list__item-quantity">2 tbsp </span><span class="ingredients-list__item-ingredient">cocoa</span></li></ul>
<ul class="method-steps__list"><li class="method-steps__list-item"><h3 class="method-steps__item-heading">step 1</h3><div class="editor-content"><p>Heat oven to 180C/160C fan.</p><div class="ad-placement"></div></div></li>
<li class="method-steps__list-item"><h3 class="method-steps__item-heading">step 2</h3><div class="editor-content"><p>Beat everything together until smooth.</p></div></li>
<li class="method-steps__list-item"><h3 class="method-steps__item-heading">step 3</h3><div class="editor-content"><p>Bake for 25 minutes.</p></div></li></ul></body></html>"#, 5, 3),
        rule("seriouseats.com", "Serious Eats", 2, &["h1.heading__title", "h1"],
            &[".structured-ingredients__list-item", ".ingredient-list li"],
            &["ol.mntl-sc-block-group--OL li p", "#structured-project__steps_1-0 li p"], &[],
            r#"<html><body><h1 class="heading__title">Crispy Roast Potatoes</h1>
<ul><li class="structured-ingredients__list-item"><p>4 pounds russet potatoes</p></li><li class="structured-ingredients__list-item"><p>1/4 cup olive oil</p></li><li class="structured-ingredients__list-item"><p>Kosher salt</p></li></ul>
<ol class="mntl-sc-block-group--OL"><li><p>Boil the potatoes for 10 minutes.</p></li><li><p>Toss with oil and roast for 40 minutes.</p></li></ol></body></html>"#, 3, 2),
        rule("simplyrecipes.com", "Simply Recipes", 2, &["h1.heading__title", "h1"],
            &[".structured-ingredients__list-item", ".ingredient"],
            &["ol.mntl-sc-block-group--OL li p", ".structured-project__steps li p"], &[],
            r#"<html><body><h1 class="heading__title">Guacamole</h1>
<ul><li class="structured-ingredients__list-item"><p>2 ripe avocados</p></li><li class="structured-ingredients__list-item"><p>1 tablespoon lime juice</p></li></ul>
<ol class="mntl-sc-block-group--OL"><li><p>Mash the avocados with the lime juice.</p></li></ol></body></html>"#, 2, 1),
        rule("epicurious.com", "Epicurious", 2, &["h1[data-testid='ContentHeaderHed']", "h1"],
            &["[data-testid='IngredientList'] .ingredient-description", "[data-testid='IngredientList'] div[class*='Description']"],
            &["[data-testid='InstructionsWrapper'] li p", "[data-testid='InstructionsWrapper'] li"], &[],
            r#"<html><body><h1 data-testid="ContentHeaderHed">Lemon Bars</h1>
<div data-testid="IngredientList"><div class="ingredient-description">1 cup flour</div><div class="ingredient-description">1/2 cup butter</div><div class="ingredient-description">2 lemons</div></div>
<div data-testid="InstructionsWrapper"><ol><li><p>Press the crust into the pan and bake for 20 minutes.</p></li><li><p>Pour the filling over and bake for 25 minutes.</p></li></ol></div></body></html>"#, 3, 2),
        rule("bonappetit.com", "Bon Appétit", 2, &["h1[data-testid='ContentHeaderHed']", "h1"],
            &["[data-testid='IngredientList'] .ingredient-description", "[data-testid='IngredientList'] div[class*='Description']"],
            &["[data-testid='InstructionsWrapper'] li p", "[data-testid='InstructionsWrapper'] li"], &[],
            r#"<html><body><h1 data-testid="ContentHeaderHed">Pasta Pomodoro</h1>
<div data-testid="IngredientList"><div class="ingredient-description">12 oz spaghetti</div><div class="ingredient-description">1 can whole tomatoes</div></div>
<div data-testid="InstructionsWrapper"><ol><li><p>Cook the pasta for 9 minutes.</p></li><li><p>Toss with the sauce.</p></li></ol></div></body></html>"#, 2, 2),
        rule("food52.com", "Food52", 2, &["h1.recipe__title", "h1"],
            &[".recipe__list--ingredients li", ".recipe-list--ingredients li"],
            &[".recipe__list--steps li", ".recipe-list--steps li"], &[],
            r#"<html><body><h1 class="recipe__title">Genius Tomato Sauce</h1>
<ul class="recipe__list--ingredients"><li>28 ounces whole tomatoes</li><li>5 tablespoons butter</li><li>1 onion, halved</li></ul>
<ol class="recipe__list--steps"><li>Combine everything in a pot.</li><li>Simmer for 45 minutes.</li></ol></body></html>"#, 3, 2),
        rule("tasty.co", "Tasty", 2, &["h1.recipe-name", "h1"],
            &[".ingredients__section li.ingredient", ".ingredients__section li"],
            &[".preparation ol li", "ol.prep-steps li"], &[".servings-display"],
            r#"<html><body><h1 class="recipe-name">One-Pot Mac and Cheese</h1>
<div class="ingredients__section"><ul><li class="ingredient">2 cups macaroni</li><li class="ingredient">2 cups milk</li><li class="ingredient">2 cups cheddar cheese</li></ul></div>
<div class="preparation"><ol><li>Boil the macaroni in the milk for 8 minutes.</li><li>Stir in the cheese.</li></ol></div></body></html>"#, 3, 2),
        // v2 (verified against the live site 2026-11-03): steps are li.direction with a "Step N" label.
        rule("delish.com", "Delish", 2, &["h1"],
            &[".ingredient-lists li", "ul.ingredient-lists li"],
            &["ul.directions li.direction", "li.direction", ".direction-lists li"], &[],
            r#"<html><body><h1>Garlic Bread</h1><ul class="ingredient-lists"><li>1 baguette</li><li>1/2 cup butter</li><li>4 cloves garlic</li></ul>
<ul class="directions"><li><ol><li class="direction"><span>Step </span><span>1</span>Mix the butter and garlic.</li><li class="direction"><span>Step </span><span>2</span>Spread on the bread and bake for 10 minutes.</li></ol></li></ul></body></html>"#, 3, 2),
        rule("jamieoliver.com", "Jamie Oliver", 1, &["h1"],
            &[".ingred-list li", ".ingredients-rich-text li"],
            &[".recipeSteps li", ".method-rich-text li"], &[],
            r#"<html><body><h1>Pea Soup</h1><ul class="ingred-list"><li>1 onion</li><li>500 g frozen peas</li><li>1 litre vegetable stock</li></ul>
<ol class="recipeSteps"><li>Soften the onion for 10 minutes.</li><li>Add the peas and stock and simmer for 5 minutes.</li></ol></body></html>"#, 3, 2),
        rule("nytimes.com", "NYT Cooking", 1, &["h1[class*='title']", "h1"],
            &["li[class*='ingredient_ingredient']", "ul[class*='ingredientList'] li"],
            &["li[class*='preparation_step'] p", "ol[class*='preparation'] li p"], &[],
            r#"<html><body><h1 class="pantry--title-display">Sheet-Pan Gnocchi</h1>
<ul><li class="ingredient_ingredient__x1"><span>1 pound shelf-stable gnocchi</span></li><li class="ingredient_ingredient__x1"><span>1 pint cherry tomatoes</span></li></ul>
<ol><li class="preparation_step__y2"><p>Heat oven to 450 degrees.</p></li><li class="preparation_step__y2"><p>Roast for 20 minutes.</p></li></ol></body></html>"#, 2, 2),
    ]
}

/// Common recipe plugins used by thousands of food blogs (tried when no site rule matches).
pub const PLUGIN_RULES: &[(&str, &str, &str)] = &[
    ("WP Recipe Maker", ".wprm-recipe-ingredient", ".wprm-recipe-instruction-text"),
    ("Tasty Recipes", ".tasty-recipes-ingredients li", ".tasty-recipes-instructions li"),
    ("Create by Mediavine", ".mv-create-ingredients li", ".mv-create-instructions li"),
    ("Zip Recipes", ".zrdn-ingredient-item", ".zrdn-instruction-item"),
    ("Microdata", "[itemprop='recipeIngredient']", "[itemprop='recipeInstructions'] li, [itemprop='recipeInstructions']"),
    ("Microdata (old)", "[itemprop='ingredients']", "[itemprop='recipeInstructions']"),
    ("Generic lists", ".ingredients li, .recipe-ingredients li, #ingredients li", ".instructions li, .directions li, .recipe-directions li, .method li, #instructions li, #directions li"),
];

pub fn host_of(url: &str) -> String {
    let u = url.trim();
    let u = u.split("://").nth(1).unwrap_or(u);
    let host = u.split(['/', '?', '#']).next().unwrap_or("").to_lowercase();
    host.trim_start_matches("www.").to_string()
}

/// The rule for a host: overrides first (newer layouts), then the built-in set.
pub fn rule_for<'a>(host: &str, overrides: &'a [SiteRule], builtin: &'a [SiteRule]) -> Option<&'a SiteRule> {
    let m = |r: &&SiteRule| host == r.domain || host.ends_with(&format!(".{}", r.domain));
    overrides.iter().find(m).or_else(|| builtin.iter().find(m))
}

#[derive(Clone, Debug)]
pub struct RuleTest {
    pub domain: String,
    pub passed: bool,
    pub detail: String,
}

pub fn test_rule(r: &SiteRule) -> RuleTest {
    let url = format!("https://www.{}/test-recipe", r.domain);
    let found = crate::importer::apply_rule_html(&r.fixture, r);
    let (ni, ns) = match &found {
        Some(d) => (d.ingredients.len(), d.steps.len()),
        None => (0, 0),
    };
    let title_ok = found.as_ref().map(|d| !d.title.is_empty()).unwrap_or(false);
    let passed = ni == r.expect_ingredients && ns == r.expect_steps && title_ok;
    let _ = url;
    RuleTest {
        domain: r.domain.clone(),
        passed,
        detail: format!(
            "{} ingredients (expected {}), {} steps (expected {}){}",
            ni,
            r.expect_ingredients,
            ns,
            r.expect_steps,
            if title_ok { "" } else { ", no title" }
        ),
    }
}

pub fn test_all(overrides: &[SiteRule]) -> Vec<RuleTest> {
    let builtin = builtin_rules();
    let mut out = vec![];
    for r in &builtin {
        let eff = overrides.iter().find(|o| o.domain == r.domain).unwrap_or(r);
        out.push(test_rule(eff));
    }
    for o in overrides {
        if !builtin.iter().any(|b| b.domain == o.domain) {
            out.push(test_rule(o));
        }
    }
    out
}

/// Load a rules update file (JSON array of rules). Rules with a fixture must pass their test.
pub fn load_update(text: &str) -> Result<(Vec<SiteRule>, Vec<String>), String> {
    let rules: Vec<SiteRule> = serde_json::from_str(text).map_err(|e| format!("Not a rules file: {}", e))?;
    let mut ok = vec![];
    let mut rejected = vec![];
    for r in rules {
        if r.domain.is_empty() || r.ingredients.is_empty() {
            rejected.push(format!("{}: incomplete rule", r.domain));
            continue;
        }
        if !r.fixture.is_empty() {
            let t = test_rule(&r);
            if !t.passed {
                rejected.push(format!("{}: failed its test ({})", r.domain, t.detail));
                continue;
            }
        }
        ok.push(r);
    }
    Ok((ok, rejected))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_builtin_rule_passes_its_fixture() {
        for t in test_all(&[]) {
            assert!(t.passed, "{}: {}", t.domain, t.detail);
        }
    }
    #[test]
    fn host_parsing() {
        assert_eq!(host_of("https://www.allrecipes.com/recipe/1/x"), "allrecipes.com");
        let b = builtin_rules();
        assert!(rule_for("cooking.nytimes.com", &[], &b).is_some());
    }
}
