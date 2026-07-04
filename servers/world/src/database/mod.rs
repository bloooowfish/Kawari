mod character;
mod friends;
mod housing;
pub use crate::housing::{
    apartment::MAX_APARTMENT_ROOM_NUMBER,
    constants::{
        DEFAULT_LOCAL_HOUSING_DIVISION, DEFAULT_LOCAL_HOUSING_LAND_FLAGS,
        DEFAULT_LOCAL_HOUSING_PLOT_INDEX, DEFAULT_LOCAL_HOUSING_PLOT_SIZE,
        DEFAULT_LOCAL_HOUSING_TERRITORY_TYPE_ID, DEFAULT_LOCAL_HOUSING_WARD_INDEX,
    },
};
pub use housing::{
    HousingEstateDetailQuery, HousingEstateExport, HousingEstateSpec, HousingEstateSummaryQueryRow,
    HousingFurnitureCounts,
};
mod linkshell;
mod mail;

mod models;
pub use models::{
    AetherCurrent, Aetheryte, Character, ClassJob, Companion, Content, Friends, GrandCompany,
    HousingEstate, HousingFurniture, Mentor, Quest, SearchInfo, Unlock, Volatile,
};

mod schema;
mod social;

use diesel::{
    Connection, QueryDsl, QueryableByName, RunQueryDsl, SqliteConnection,
    connection::SimpleConnection, prelude::*, sql_query, sql_types::BigInt,
};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};
use kawari::{
    common::ObjectId,
    constants::{
        COMPLETED_LEGACY_QUEST_BITMASK_SIZE, COMPLETED_LEVEQUEST_BITMASK_SIZE,
        UNLOCKED_MAP_MARKERS_BITMASK_SIZE,
    },
};

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

pub struct WorldDatabase {
    connection: SqliteConnection,
}

#[derive(QueryableByName)]
struct SchemaColumnCount {
    #[diesel(sql_type = BigInt)]
    count: i64,
}

fn zero_bitmask_json(size: usize) -> String {
    serde_json::to_string(&vec![0u8; size]).expect("zero bitmask should serialize")
}

fn quest_column_exists(connection: &mut SqliteConnection, column: &str) -> bool {
    let query =
        format!("SELECT COUNT(*) AS count FROM pragma_table_info('quest') WHERE name = '{column}'");
    sql_query(query)
        .get_result::<SchemaColumnCount>(connection)
        .map(|result| result.count > 0)
        .unwrap_or_default()
}

fn ensure_legacy_quest_completion_columns(connection: &mut SqliteConnection) {
    for (column, size) in [
        ("completed_legacy", COMPLETED_LEGACY_QUEST_BITMASK_SIZE),
        ("unlocked_map_markers", UNLOCKED_MAP_MARKERS_BITMASK_SIZE),
        ("completed_levequests", COMPLETED_LEVEQUEST_BITMASK_SIZE),
    ] {
        if quest_column_exists(connection, column) {
            continue;
        }

        let default_value = zero_bitmask_json(size);
        connection
            .batch_execute(&format!(
                "ALTER TABLE `quest` ADD COLUMN `{column}` TEXT NOT NULL DEFAULT '{default_value}'"
            ))
            .expect("failed to add legacy quest completion column");
    }
}

impl Default for WorldDatabase {
    fn default() -> Self {
        Self::new()
    }
}

impl WorldDatabase {
    pub fn new() -> Self {
        Self::new_at("world.db")
    }

    pub fn new_at(database_url: &str) -> Self {
        let mut connection =
            SqliteConnection::establish(database_url).expect("Failed to open database!");

        connection.run_pending_migrations(MIGRATIONS).unwrap();
        ensure_legacy_quest_completion_columns(&mut connection);

        Self { connection }
    }

    fn generate_content_id() -> u32 {
        fastrand::u32(..)
    }

    fn generate_actor_id() -> ObjectId {
        ObjectId(fastrand::u32(..))
    }

    /// returns
    pub fn find_service_account(&mut self, for_content_id: u64) -> u64 {
        use schema::character::dsl::*;

        character
            .filter(content_id.eq(for_content_id as i64))
            .select(service_account_id)
            .first::<i64>(&mut self.connection)
            .unwrap_or_default() as u64
    }

    pub fn do_cleanup_tasks(&mut self) {
        // Ensure the most volatile aspects of the db are reset to a clean state.
        // We expect these to be "offline" as the initial state elsewhere for things like the online player count and friend lists to function correctly.
        {
            use schema::volatile::dsl::*;

            diesel::update(volatile)
                .set(is_online.eq(false))
                .execute(&mut self.connection)
                .unwrap();
        }

        // Clean up orphaned linkshells with no members that were missed somehow. This should theoretically not happen without manual database edits.
        {
            use schema::linkshell_members::dsl::*;

            for (orphaned_linkshell_id, _) in self.find_all_linkshells() {
                if let Ok(members) = linkshell_members
                    .select(models::LinkshellMembers::as_select())
                    .filter(linkshell_id.eq(orphaned_linkshell_id as i64))
                    .load(&mut self.connection)
                    && members.is_empty()
                {
                    tracing::info!(
                        "Found orphaned linkshell {orphaned_linkshell_id} with zero members, cleaning it up now."
                    );
                    self.remove_linkshell(orphaned_linkshell_id);
                }
            }

            // TODO: Auto-promote new owners in linkshells that don't have owners, which should theoretically not happen without manual database edits.
        }
    }
}

#[declare_sql_function]
extern "SQL" {
    fn datetime() -> diesel::sql_types::Text;
}

#[declare_sql_function]
extern "SQL" {
    fn unixepoch() -> diesel::sql_types::BigInt;
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use diesel::{
        QueryableByName, RunQueryDsl, connection::SimpleConnection, sql_query, sql_types::Text,
    };

    use super::WorldDatabase;

    #[derive(QueryableByName)]
    struct QuestCompletionColumns {
        #[diesel(sql_type = Text)]
        completed_legacy: String,
        #[diesel(sql_type = Text)]
        unlocked_map_markers: String,
        #[diesel(sql_type = Text)]
        completed_levequests: String,
    }

    fn temp_database_path(test_name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "kawari-world-{test_name}-{}-{}.db",
            std::process::id(),
            fastrand::u64(..)
        ));
        path
    }

    #[test]
    fn world_migrations_upgrade_legacy_quest_table_with_completion_fields() {
        let database_path = temp_database_path("legacy-quest-completion-fields");
        let database_url = database_path.to_string_lossy().into_owned();

        {
            let mut database = WorldDatabase::new_at(&database_url);
            database
                .connection
                .batch_execute(
                    r#"
                    DROP TABLE `quest`;
                    CREATE TABLE `quest`(
                        `content_id` BIGINT NOT NULL PRIMARY KEY,
                        `completed` TEXT NOT NULL,
                        `active` TEXT NOT NULL
                    );
                    INSERT INTO `quest` (`content_id`, `completed`, `active`)
                        VALUES (1, '[]', '[]');
                    "#,
                )
                .expect("legacy test schema should be created");
        }

        let mut database = WorldDatabase::new_at(&database_url);
        let columns = sql_query(
            "SELECT completed_legacy, unlocked_map_markers, completed_levequests \
             FROM quest WHERE content_id = 1",
        )
        .get_result::<QuestCompletionColumns>(&mut database.connection)
        .expect("quest completion columns should exist after migration");

        let completed_legacy: Vec<u8> =
            serde_json::from_str(&columns.completed_legacy).expect("legacy mask should be JSON");
        let unlocked_map_markers: Vec<u8> = serde_json::from_str(&columns.unlocked_map_markers)
            .expect("map marker mask should be JSON");
        let completed_levequests: Vec<u8> = serde_json::from_str(&columns.completed_levequests)
            .expect("levequest mask should be JSON");

        assert_eq!(completed_legacy, vec![0; 40]);
        assert_eq!(unlocked_map_markers, vec![0; 64]);
        assert_eq!(completed_levequests, vec![0; 226]);

        fs::remove_file(database_path).ok();
    }
}
