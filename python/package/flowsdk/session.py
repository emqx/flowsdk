"""Application-owned checkpoint stores. Storage errors propagate unchanged."""

import sqlite3
from typing import Optional, Protocol


class ClientSessionStore(Protocol):
    """Use one writer per key. resume never deletes; delete is idempotent."""
    def create(self, key: str, checkpoint: bytes) -> None: ...
    def resume(self, key: str) -> Optional[bytes]: ...
    def update(self, key: str, checkpoint: bytes) -> None: ...
    def delete(self, key: str) -> None: ...


class SqliteSessionStore:
    """A synchronous, transactional store for planned restarts.

    Call outside engine callbacks. Keys should include broker/client/account
    identity. SQLite FULL synchronous commits must succeed before restarting.
    An existing key on create raises IntegrityError; missing update raises KeyError.
    This stores opaque bytes and does not make application effects transactional.
    """
    def __init__(self, path):
        self._db = sqlite3.connect(path)
        try:
            self._db.execute("PRAGMA synchronous=FULL")
            self._db.execute("CREATE TABLE IF NOT EXISTS sessions (key TEXT PRIMARY KEY, checkpoint BLOB NOT NULL)")
            self._db.commit()
        except BaseException:
            self._db.close()
            raise

    def create(self, key: str, checkpoint: bytes) -> None:
        with self._db:
            self._db.execute("INSERT INTO sessions VALUES (?, ?)", (key, bytes(checkpoint)))

    def resume(self, key: str) -> Optional[bytes]:
        row = self._db.execute("SELECT checkpoint FROM sessions WHERE key = ?", (key,)).fetchone()
        return bytes(row[0]) if row is not None else None

    def update(self, key: str, checkpoint: bytes) -> None:
        with self._db:
            result = self._db.execute("UPDATE sessions SET checkpoint = ? WHERE key = ?", (bytes(checkpoint), key))
            if result.rowcount != 1:
                raise KeyError(key)

    def delete(self, key: str) -> None:
        with self._db:
            self._db.execute("DELETE FROM sessions WHERE key = ?", (key,))

    def close(self):
        self._db.close()

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()
