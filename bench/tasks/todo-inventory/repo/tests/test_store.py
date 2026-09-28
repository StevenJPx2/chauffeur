import unittest

from app.core.store import Store


class StoreTest(unittest.TestCase):
    def test_missing_file_is_empty(self):
        store = Store()
        store.load("does-not-exist.json")
        self.assertEqual(store.items(), [])

    # TODO: cover save() round-trips


if __name__ == "__main__":
    unittest.main()
