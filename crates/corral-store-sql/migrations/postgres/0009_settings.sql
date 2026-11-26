create table settings (
    key text primary key,
    value text not null,
    updated_at timestamptz not null
);

create table member_allowances (
    user_id uuid primary key references users on delete cascade,
    max_sandboxes bigint not null
);
