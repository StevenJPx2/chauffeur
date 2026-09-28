from datetime import date
from decimal import Decimal

from shop.models import DiscountCode, OrderLine
from shop.orders import place_order

from .helpers import StoreTestCase

TODAY = date(2026, 3, 1)


class DiscountTest(StoreTestCase):
    def test_percent_code(self):
        self.store.add_code(DiscountCode("SPRING10", "percent", Decimal("10"), date(2026, 3, 31)))
        order = place_order(self.store, "alice", [OrderLine("MUG", 2)], code="SPRING10", today=TODAY)
        self.assertEqual(order.total, Decimal("22.50"))

    def test_fixed_code(self):
        self.store.add_code(DiscountCode("FIVEOFF", "fixed", Decimal("5.00"), date(2026, 3, 31)))
        order = place_order(self.store, "alice", [OrderLine("MUG", 1)], code="FIVEOFF", today=TODAY)
        self.assertEqual(order.total, Decimal("7.50"))
