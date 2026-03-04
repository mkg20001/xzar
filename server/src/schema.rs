// @generated automatically by Diesel CLI.

diesel::table! {
    drvs (drv_id) {
        #[max_length = 32]
        drv_id -> Varchar,
        drv_full -> Varchar,
        nar_hash -> Varchar,
        nar_size -> Int8,
        file_hash -> Varchar,
        file_size -> Int8,
        deriver -> Nullable<Varchar>,
        sig -> Nullable<Varchar>,
        refs -> Array<Nullable<Varchar>>,
        nar_comp -> Nullable<Varchar>,
        nar_file -> Varchar,
        nar_file_storage -> Varchar,
        gc -> Bool,
        created -> Timestamp,
        last_fetched -> Nullable<Timestamp>,
    }
}

diesel::table! {
    locks (id) {
        id -> Int4,
        expires -> Timestamp,
        owner -> Int4,
    }
}

diesel::table! {
    pins (id) {
        id -> Int4,
        #[max_length = 128]
        name -> Varchar,
        #[max_length = 1024]
        description -> Nullable<Varchar>,
        created -> Timestamp,
        expires -> Nullable<Timestamp>,
        abandoned -> Bool,
        leave_after_abandon -> Nullable<Int8>,
    }
}

diesel::table! {
    drv_locks (drv_id, lock_id) {
        #[max_length = 32]
        drv_id -> Varchar,
        lock_id -> Int4,
    }
}

diesel::table! {
    drv_pins (drv_id, pin_id) {
        #[max_length = 32]
        drv_id -> Varchar,
        pin_id -> Int4,
    }
}

diesel::table! {
    users (id) {
        id -> Int4,
        #[max_length = 128]
        name -> Varchar,
        is_admin -> Bool,
        created -> Timestamp,
        #[max_length = 256]
        email -> Nullable<Varchar>,
    }
}

diesel::table! {
    tokens (id) {
        id -> Int4,
        user_id -> Nullable<Int4>,
        #[max_length = 128]
        token_hash -> Varchar,
        is_system -> Bool,
        #[max_length = 256]
        description -> Nullable<Varchar>,
        created -> Timestamp,
    }
}

diesel::joinable!(drv_locks -> drvs (drv_id));
diesel::joinable!(drv_locks -> locks (lock_id));
diesel::joinable!(drv_pins -> drvs (drv_id));
diesel::joinable!(drv_pins -> pins (pin_id));
diesel::joinable!(tokens -> users (user_id));

diesel::allow_tables_to_appear_in_same_query!(drvs, drv_locks, drv_pins, locks, pins, users, tokens,);
