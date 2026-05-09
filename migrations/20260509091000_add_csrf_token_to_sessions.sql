ALTER TABLE sessions
    ALTER COLUMN user_id DROP NOT NULL,
    ADD COLUMN csrf_token TEXT NOT NULL DEFAULT '';

UPDATE sessions
SET csrf_token = id::text
WHERE csrf_token = '';
