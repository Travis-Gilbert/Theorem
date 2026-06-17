create extension if not exists pgcrypto;

do $$
begin
    if not exists (select 1 from pg_type where typname = 'dispatch_state') then
        create type dispatch_state as enum ('pending','claimed','running','done','failed','dead');
    end if;
    if not exists (select 1 from pg_type where typname = 'dispatch_head') then
        create type dispatch_head as enum ('claude','codex','either');
    end if;
end $$;

create table if not exists dispatch_jobs (
    job_id           text primary key,
    title            text not null,
    repo             text,
    spec_ref         text,
    spec_inline      text,
    target_head      dispatch_head not null default 'either',
    priority         smallint not null default 100,
    state            dispatch_state not null default 'pending',
    not_before       timestamptz not null default now(),
    claimed_by       text,
    claimed_at       timestamptz,
    lease_expires_at timestamptz,
    attempts         smallint not null default 0,
    max_attempts     smallint not null default 3,
    result           jsonb,
    source_task_id   text,
    created_at       timestamptz not null default now(),
    updated_at       timestamptz not null default now(),
    constraint dispatch_jobs_attempts_nonnegative check (attempts >= 0),
    constraint dispatch_jobs_max_attempts_positive check (max_attempts > 0),
    constraint dispatch_jobs_has_spec check (
        nullif(btrim(coalesce(spec_ref, '')), '') is not null
        or nullif(btrim(coalesce(spec_inline, '')), '') is not null
    )
);

create index if not exists dispatch_jobs_claimable
    on dispatch_jobs (priority, not_before)
    where state = 'pending';

create index if not exists dispatch_jobs_leased
    on dispatch_jobs (lease_expires_at)
    where state in ('claimed','running');
