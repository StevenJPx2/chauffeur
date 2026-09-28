from decimal import Decimal

from shop.storage import NotFound, Store

from .helpers import StoreTestCase


class StorageTest(StoreTestCase):
    def test_products_persist(self):
        reopened = Store(self.path)
        self.assertEqual(reopened.get_product("PEN").price, Decimal("1.99"))

    def test_unknown_product(self):
        with self.assertRaises(NotFound):
            self.store.get_product("NOPE")
