//! `get_preferences`, `update_my_preferences`, and the person-only
//! `POST /v1/customer/me/memories/{memory_id}/undo`: what the agent remembers about a shopper,
//! always visible to them and undoable with one tap. Only facts about the shopper themselves
//! are kept; never a gift recipient's, never payment or health data.

mod get_preferences;
mod undo_memory;
mod update_my_preferences;

pub use get_preferences::*;
pub use undo_memory::*;
pub use update_my_preferences::*;

/// How many memories one call may add or remove.
const MAX_ITEMS: usize = 5;

#[cfg(test)]
mod tests {
    use acom_contract::{PersonOp, WriteOp};
    use acom_core::enums::PreferenceKey;

    use super::*;

    /// Integration: needs a seeded database (`DATABASE_URL`). Run with `-- --ignored`.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn remembers_and_forgets() {
        let cx = crate::testdb::cx("cus_jamie").await;
        let planned = UpdateMyPreferences
            .preview(
                &cx,
                UpdateMyPreferencesInput {
                    add: Some(vec![AddMemory {
                        key: PreferenceKey::TravelPriority,
                        value: "weight and packed size".into(),
                        source: MemorySource::Conversation,
                    }]),
                    remove: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            planned.plan.add[0].text,
            "Remembered: weight and packed size matter to you when traveling"
        );
        let mut tx = cx.begin().await.unwrap();
        let done = UpdateMyPreferences
            .commit(&mut tx, &planned.plan)
            .await
            .unwrap();
        let again = UpdateMyPreferences
            .commit(&mut tx, &planned.plan)
            .await
            .unwrap();
        assert_eq!(done.id, again.id);
        let out = UndoMemory
            .run(
                &mut tx,
                UndoMemoryInput {
                    memory_id: done.id.clone(),
                },
            )
            .await
            .unwrap();
        assert_eq!(out["ok"], true);
        let e = UndoMemory
            .run(&mut tx, UndoMemoryInput { memory_id: done.id })
            .await
            .unwrap_err();
        assert_eq!(e.code, "memory_not_found");
    }
}
