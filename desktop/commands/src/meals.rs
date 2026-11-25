//! The meal plan: recipes on days, as breakfast, lunch, dinner or a snack.

use crate::{mutate, Shared};
use recipe_box::model::{self, Meal, MealType};
use recipe_box::tracker::diary;
use serde::Serialize;

#[derive(Serialize)]
pub struct MealOut {
    #[serde(flatten)]
    meal: Meal,
    label: Option<usize>,
    photo: String,
    seed: u32,
    /** the diary has a live entry for this meal's recipe on its day */
    eaten: bool,
}

/// Meals between two days (inclusive), with what the page needs to draw each recipe.
pub fn meals(state: &Shared, from: i32, to: i32) -> Result<Vec<MealOut>, String> {
    let mut guard = state.lock().unwrap();
    let st = &mut *guard;
    let lib = &st.lib;
    let range: Vec<&Meal> = lib.meals.iter().filter(|m| m.day >= from && m.day <= to).collect();
    let eaten = diary::eaten(&mut st.journal, &range, |id| lib.recipe(id).map(|r| r.uid.clone()));
    let mut v: Vec<MealOut> = range
        .into_iter()
        .map(|m| {
            let r = m.recipe_id.and_then(|id| lib.recipe(id));
            MealOut {
                label: r.and_then(|r| model::recipe_label(&lib.label_colors, &r.categories).1),
                photo: r.map(|r| r.photo.clone()).unwrap_or_default(),
                seed: r.map_or(1, |r| r.photo_seed),
                eaten: eaten.contains(&m.id),
                meal: m.clone(),
            }
        })
        .collect();
    v.sort_by_key(|m| (m.meal.day, m.meal.kind as u8, m.meal.id));
    Ok(v)
}

/// The first meal of the day that's still free (the classic app's choice for a recipe dropped
/// on a day), or a snack when all four are taken.
pub fn free_kind(meals: &[Meal], day: i32) -> MealType {
    MealType::ALL.iter().copied().find(|k| !meals.iter().any(|m| m.day == day && m.kind == *k)).unwrap_or(MealType::Snack)
}

/// Move a meal to another day and/or make it another meal.
pub fn update_meal(state: &Shared, id: u64, day: Option<i32>, kind: Option<MealType>) -> Result<(), String> {
    mutate(state, |st| {
        let m = st.lib.meals.iter_mut().find(|m| m.id == id).ok_or("That meal isn't in the plan any more.")?;
        if let Some(d) = day {
            m.day = d;
        }
        if let Some(k) = kind {
            m.kind = k;
        }
        m.modified = model::now_secs();
        Ok(())
    })
}

/// Take meals off the plan: one by id, or every meal between two days (a Version History
/// snapshot is taken first for that). Returns what went, so Undo can put it back.
pub fn remove_meals(state: &Shared, ids: Vec<u64>, from: Option<i32>, to: Option<i32>) -> Result<Vec<Meal>, String> {
    mutate(state, |st| {
        let range = from.zip(to);
        if range.is_some() {
            st.store.snapshot(&st.lib, "before clearing meals")?;
        }
        let gone = |m: &Meal| ids.contains(&m.id) || range.is_some_and(|(f, t)| m.day >= f && m.day <= t);
        let removed: Vec<Meal> = st.lib.meals.iter().filter(|m| gone(m)).cloned().collect();
        st.lib.meals.retain(|m| !gone(m));
        Ok(removed)
    })
}

/// Undo for a removal: the meals go back as they were.
pub fn restore_meals(state: &Shared, meals: Vec<Meal>) -> Result<(), String> {
    mutate(state, |st| {
        let t = model::now_secs();
        for mut m in meals {
            if st.lib.meals.iter().any(|x| x.id == m.id) {
                m.id = st.lib.new_id();
            }
            m.modified = t;
            st.lib.meals.push(m);
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use recipe_box::model::{Meal, MealType};

    #[test]
    fn a_dropped_recipe_takes_the_first_free_meal() {
        let m = |day, kind| Meal { day, kind, ..Default::default() };
        assert_eq!(super::free_kind(&[], 5), MealType::Breakfast);
        assert_eq!(super::free_kind(&[m(5, MealType::Breakfast), m(6, MealType::Lunch)], 5), MealType::Lunch);
        let full = [m(5, MealType::Breakfast), m(5, MealType::Lunch), m(5, MealType::Dinner), m(5, MealType::Snack)];
        assert_eq!(super::free_kind(&full, 5), MealType::Snack);
    }
}
