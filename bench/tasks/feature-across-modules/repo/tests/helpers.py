import tempfile
import unittest
from decimal import Decimal
from pathlib import Path

from shop.models import Product
from shop.storage import Store


class StoreTestCase(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.path = Path(self._tmp.name) / "shop.json"
        self.store = Store(self.path)
        self.store.add_product(Product("MUG", "Coffee mug", Decimal("12.50")))
        self.store.add_product(Product("PEN", "Gel pen", Decimal("1.99")))

    def tearDown(self):
        self._tmp.cleanup()
