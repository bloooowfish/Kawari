CREATE TABLE IF NOT EXISTS `housing_estates` (
    `land_ident` BIGINT NOT NULL PRIMARY KEY,
    `house_id` BIGINT NOT NULL UNIQUE,
    `territory_type_id` INTEGER NOT NULL,
    `world_id` INTEGER NOT NULL,
    `ward_index` INTEGER NOT NULL,
    `division` INTEGER NOT NULL DEFAULT 0,
    `plot_index` INTEGER NOT NULL,
    `room_number` INTEGER NOT NULL DEFAULT 0,
    `is_apartment` BOOL NOT NULL DEFAULT 0,
    `owner_content_id` BIGINT,
    `owner_name` TEXT NOT NULL DEFAULT '',
    `plot_size` INTEGER NOT NULL DEFAULT 0,
    `flags` INTEGER NOT NULL DEFAULT 0,
    `estate_name` TEXT NOT NULL DEFAULT '',
    `greeting` TEXT NOT NULL DEFAULT '',
    `exterior_json` TEXT NOT NULL DEFAULT '{}',
    `interior_json` TEXT NOT NULL DEFAULT '{}',
    `light_level` INTEGER NOT NULL DEFAULT 0,
    `created_at` BIGINT NOT NULL DEFAULT 0,
    `updated_at` BIGINT NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS `housing_furniture` (
    `land_ident` BIGINT NOT NULL,
    `container_type` INTEGER NOT NULL,
    `slot` INTEGER NOT NULL,
    `item_id` BIGINT NOT NULL,
    `catalog_id` INTEGER NOT NULL,
    `stain` INTEGER NOT NULL DEFAULT 0,
    `placed` BOOL NOT NULL DEFAULT 0,
    `pos_x` REAL NOT NULL DEFAULT 0,
    `pos_y` REAL NOT NULL DEFAULT 0,
    `pos_z` REAL NOT NULL DEFAULT 0,
    `rotation` REAL NOT NULL DEFAULT 0,
    `created_by_content_id` BIGINT,
    `updated_at` BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (`land_ident`, `container_type`, `slot`)
);

CREATE INDEX IF NOT EXISTS `idx_housing_estates_owner`
    ON `housing_estates` (`owner_content_id`);

CREATE INDEX IF NOT EXISTS `idx_housing_estates_zone_plot`
    ON `housing_estates` (`territory_type_id`, `world_id`, `ward_index`, `division`, `plot_index`);

CREATE INDEX IF NOT EXISTS `idx_housing_furniture_land_placed`
    ON `housing_furniture` (`land_ident`, `placed`);
