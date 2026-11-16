-- Migrations run without lock_timeout, so instances starting together can wait on the
-- migration lock; this fails the migration when it is set.
DO $$
BEGIN
  IF current_setting('lock_timeout') <> '0' THEN
    RAISE EXCEPTION 'migrations ran with lock_timeout %', current_setting('lock_timeout');
  END IF;
END
$$;
