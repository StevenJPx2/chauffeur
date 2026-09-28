import contextlib
import io
import json
import tempfile
from datetime import date
from decimal import Decimal
from pathlib import Path

from shop import cli
from shop.models import DiscountCode, OrderLine, Product
from shop.orders import place_order
from shop.pricing import DiscountError
from shop.storage import NotFound, Store

from .helpers import StoreTestCase

TODAY = date(2026, 3, 1)


def run(*argv):
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = cli.main(list(argv))
    return code, out.getvalue(), err.getvalue()


class DiscountEdgeCaseTest(StoreTestCase):
    def add(self, code, kind, value, expires=date(2026, 3, 31)):
        self.store.add_code(DiscountCode(code, kind, Decimal(value), expires))

    def order(self, customer, lines, code, today=TODAY, store=None):
        return place_order(store or self.store, customer, lines, code=code, today=today)

    def test_valid_on_expiry_day(self):
        self.add("LAST", "percent", "10", expires=TODAY)
        self.assertEqual(self.order("alice", [OrderLine("MUG", 1)], "LAST").total, Decimal("11.25"))

    def test_expired_code(self):
        self.add("OLD", "percent", "10", expires=date(2026, 2, 28))
        with self.assertRaises(DiscountError):
            self.order("alice", [OrderLine("MUG", 1)], "OLD")
        self.assertEqual(self.store.orders_for("alice"), [])

    def test_unknown_code(self):
        with self.assertRaises(DiscountError):
            self.order("alice", [OrderLine("MUG", 1)], "NOSUCHCODE")
        self.assertEqual(self.store.orders_for("alice"), [])

    def test_same_customer_cannot_reuse(self):
        self.add("ONCE", "fixed", "2.00")
        self.order("alice", [OrderLine("MUG", 1)], "ONCE")
        with self.assertRaises(DiscountError):
            self.order("alice", [OrderLine("MUG", 1)], "ONCE")
        self.assertEqual(len(self.store.orders_for("alice")), 1)

    def test_other_customer_can_use(self):
        self.add("ONCE", "fixed", "2.00")
        self.order("alice", [OrderLine("MUG", 1)], "ONCE")
        self.assertEqual(self.order("bob", [OrderLine("MUG", 1)], "ONCE").total, Decimal("10.50"))

    def test_usage_persists_across_reload(self):
        self.add("ONCE", "percent", "50")
        self.order("alice", [OrderLine("MUG", 1)], "ONCE")
        reopened = Store(self.path)
        with self.assertRaises(DiscountError):
            self.order("alice", [OrderLine("MUG", 1)], "ONCE", store=reopened)

    def test_code_persists_across_reload(self):
        self.add("KEEP", "fixed", "1.00")
        reopened = Store(self.path)
        self.assertEqual(self.order("carol", [OrderLine("MUG", 1)], "KEEP", store=reopened).total, Decimal("11.50"))

    def test_failed_order_does_not_consume_code(self):
        self.add("ONCE", "fixed", "2.00")
        with self.assertRaises(NotFound):
            self.order("alice", [OrderLine("NOPE", 1)], "ONCE")
        self.assertEqual(self.order("alice", [OrderLine("MUG", 1)], "ONCE").total, Decimal("10.50"))

    def test_case_insensitive(self):
        self.add("Spring10", "percent", "10")
        self.assertEqual(self.order("alice", [OrderLine("MUG", 1)], "spring10").total, Decimal("11.25"))
        with self.assertRaises(DiscountError):
            self.order("alice", [OrderLine("MUG", 1)], "SPRING10")

    def test_fixed_larger_than_total_clamps_to_zero(self):
        self.add("BIG", "fixed", "50.00")
        self.assertEqual(self.order("alice", [OrderLine("PEN", 2)], "BIG").total, Decimal("0.00"))

    def test_percent_discount_rounds_to_cents(self):
        self.store.add_product(Product("CLIP", "Paper clip", Decimal("0.50")))
        self.add("FIVE", "percent", "5")
        # 5% of 0.50 is 0.025, which rounds half-up to 0.03
        self.assertEqual(self.order("alice", [OrderLine("CLIP", 1)], "FIVE").total, Decimal("0.47"))
        self.add("FIFTEEN", "percent", "15")
        # 15% of 12.34 is 1.851 -> 1.85
        self.store.add_product(Product("PAD", "Notepad", Decimal("12.34")))
        order = self.order("alice", [OrderLine("PAD", 1)], "FIFTEEN")
        self.assertEqual(order.total, Decimal("10.49"))
        self.assertEqual(order.total.as_tuple().exponent, -2)

    def test_hundred_percent(self):
        self.add("FREE", "percent", "100")
        self.assertEqual(self.order("alice", [OrderLine("MUG", 3)], "FREE").total, Decimal("0.00"))

    def test_no_code_unchanged(self):
        self.assertEqual(self.order("alice", [OrderLine("MUG", 1)], None).total, Decimal("12.50"))

    def test_store_file_without_codes_section(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        path = Path(tmp.name) / "legacy.json"
        path.write_text(json.dumps({"products": {"MUG": {"name": "Coffee mug", "price": "12.50"}}, "orders": []}))
        legacy = Store(path)
        legacy.add_code(DiscountCode("NEW", "fixed", Decimal("2.50"), date(2026, 12, 31)))
        self.assertEqual(self.order("alice", [OrderLine("MUG", 1)], "NEW", store=legacy).total, Decimal("10.00"))


class DiscountCliTest(StoreTestCase):
    def cli(self, *argv):
        return run("--store", str(self.path), *argv)

    def test_add_percent_code_and_checkout(self):
        code, _, err = self.cli("add-code", "TENOFF", "--percent", "10", "--expires", "2999-12-31")
        self.assertEqual(code, 0, err)
        code, out, err = self.cli("checkout", "--customer", "dana", "--item", "MUG:2", "--code", "tenoff")
        self.assertEqual(code, 0, err)
        self.assertIn("22.50", out)

    def test_add_fixed_code_and_checkout(self):
        code, _, err = self.cli("add-code", "FIVE", "--fixed", "5.00", "--expires", "2999-12-31")
        self.assertEqual(code, 0, err)
        code, out, err = self.cli("checkout", "--customer", "dana", "--item", "MUG", "--code", "FIVE")
        self.assertEqual(code, 0, err)
        self.assertIn("7.50", out)

    def test_reuse_via_cli_is_clean_error(self):
        self.cli("add-code", "ONCE", "--fixed", "1.00", "--expires", "2999-12-31")
        self.assertEqual(self.cli("checkout", "--customer", "dana", "--item", "MUG", "--code", "ONCE")[0], 0)
        code, out, err = self.cli("checkout", "--customer", "dana", "--item", "MUG", "--code", "ONCE")
        self.assertEqual(code, 1)
        self.assertNotIn("Traceback", err)
        self.assertTrue(err.strip())
        self.assertEqual(len(Store(self.path).orders_for("dana")), 1)

    def test_expired_via_cli(self):
        self.cli("add-code", "GONE", "--percent", "10", "--expires", "2000-01-01")
        code, _, err = self.cli("checkout", "--customer", "dana", "--item", "MUG", "--code", "GONE")
        self.assertEqual(code, 1)
        self.assertTrue(err.strip())

    def test_unknown_code_via_cli(self):
        code, _, err = self.cli("checkout", "--customer", "dana", "--item", "MUG", "--code", "NOPE")
        self.assertEqual(code, 1)
        self.assertTrue(err.strip())
        self.assertEqual(Store(self.path).orders_for("dana"), [])
