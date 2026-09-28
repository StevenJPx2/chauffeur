import argparse
import sys
from datetime import date
from decimal import Decimal, InvalidOperation

from .models import DiscountCode, OrderLine, Product
from .orders import place_order
from .pricing import PricingError
from .storage import NotFound, Store


def parse_item(text):
    sku, sep, qty = text.partition(":")
    if not sep:
        return OrderLine(sku, 1)
    try:
        return OrderLine(sku, int(qty))
    except ValueError:
        raise argparse.ArgumentTypeError(f"bad quantity in {text!r}")


def parse_price(text):
    try:
        return Decimal(text)
    except InvalidOperation:
        raise argparse.ArgumentTypeError(f"bad price {text!r}")


def parse_date(text):
    try:
        return date.fromisoformat(text)
    except ValueError:
        raise argparse.ArgumentTypeError(f"bad date {text!r}, expected YYYY-MM-DD")


def build_parser():
    parser = argparse.ArgumentParser(prog="shop")
    parser.add_argument("--store", default="shop.json", help="path to the JSON store")
    sub = parser.add_subparsers(dest="command", required=True)

    add = sub.add_parser("add-product", help="add or update a product")
    add.add_argument("sku")
    add.add_argument("name")
    add.add_argument("price", type=parse_price)

    code = sub.add_parser("add-code", help="add or update a discount code")
    code.add_argument("code")
    kind = code.add_mutually_exclusive_group(required=True)
    kind.add_argument("--percent", type=parse_price)
    kind.add_argument("--fixed", type=parse_price)
    code.add_argument("--expires", type=parse_date, required=True, help="last valid day, YYYY-MM-DD")

    checkout = sub.add_parser("checkout", help="place an order")
    checkout.add_argument("--customer", required=True)
    checkout.add_argument("--item", action="append", type=parse_item, required=True, help="SKU[:QTY]")
    checkout.add_argument("--code", help="discount code")

    orders = sub.add_parser("orders", help="list a customer's orders")
    orders.add_argument("--customer", required=True)
    return parser


def main(argv=None):
    args = build_parser().parse_args(argv)
    store = Store(args.store)
    try:
        if args.command == "add-product":
            store.add_product(Product(args.sku, args.name, args.price))
            print(f"Saved {args.sku}")
        elif args.command == "add-code":
            if args.percent is not None:
                discount = DiscountCode(args.code, "percent", args.percent, args.expires)
            else:
                discount = DiscountCode(args.code, "fixed", args.fixed, args.expires)
            store.add_code(discount)
            print(f"Saved code {args.code}")
        elif args.command == "checkout":
            order = place_order(store, args.customer, args.item, code=args.code)
            if order.code:
                print(f"Discount {order.code}: -{order.discount}")
            print(f"Order #{order.id} total: {order.total}")
        elif args.command == "orders":
            for order in store.orders_for(args.customer):
                print(f"#{order.id} {order.placed_on} {order.total}")
    except (NotFound, PricingError) as exc:
        print(f"error: {exc.args[0] if exc.args else exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
