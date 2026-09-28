from datetime import date
from decimal import Decimal

from shop.models import OrderLine
from shop.orders import place_order
from shop.pricing import PricingError, money, subtotal

from .helpers import StoreTestCase


class PricingTest(StoreTestCase):
    def test_money_rounds_half_up(self):
        self.assertEqual(money(Decimal("0.125")), Decimal("0.13"))
        self.assertEqual(money(Decimal("0.124")), Decimal("0.12"))

    def test_subtotal(self):
        lines = [OrderLine("MUG", 2), OrderLine("PEN", 3)]
        self.assertEqual(subtotal(self.store, lines), Decimal("30.97"))

    def test_rejects_zero_quantity(self):
        with self.assertRaises(PricingError):
            subtotal(self.store, [OrderLine("MUG", 0)])

    def test_place_order_records_it(self):
        order = place_order(self.store, "alice", [OrderLine("MUG", 1)], today=date(2026, 3, 1))
        self.assertEqual(order.total, Decimal("12.50"))
        self.assertEqual([o.id for o in self.store.orders_for("alice")], [order.id])
