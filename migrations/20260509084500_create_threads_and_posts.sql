CREATE TABLE threads (
    id BIGSERIAL PRIMARY KEY,
    category_id BIGINT NOT NULL REFERENCES categories(id) ON DELETE CASCADE,
    author_id BIGINT NOT NULL REFERENCES users(id),
    title TEXT NOT NULL,
    slug TEXT NOT NULL,
    is_locked BOOLEAN NOT NULL DEFAULT FALSE,
    is_pinned BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_activity_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX threads_category_activity_idx
    ON threads (category_id, is_pinned DESC, last_activity_at DESC, id DESC);
CREATE INDEX threads_author_id_idx ON threads (author_id);
CREATE INDEX threads_slug_idx ON threads (slug);

CREATE TABLE posts (
    id BIGSERIAL PRIMARY KEY,
    thread_id BIGINT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    author_id BIGINT NOT NULL REFERENCES users(id),
    body TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at TIMESTAMPTZ NULL
);

CREATE INDEX posts_thread_created_idx ON posts (thread_id, created_at ASC, id ASC);
CREATE INDEX posts_author_id_idx ON posts (author_id);
