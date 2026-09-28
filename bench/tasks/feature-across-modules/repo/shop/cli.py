import argparse
import sys
from decimal import Decimal, InvalidOperation

from .models import OrderLine, Product
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


def build_parser():
    parser = argparse.ArgumentParser(prog="shop")
    parser.add_argument("--store", default="shop.json", help="path to the JSON store")
    sub = parser.add_subparsers(dest="command", required=True)

    add = sub.add_parser("add-product", help="add or update a product")
    add.add_argument("sku")
    add.add_argument("name")
    add.add_argument("price", type=parse_price)

    checkout = sub.add_parser("checkout", help="place an order")
    checkout.add_argument("--customer", required=True)
    checkout.add_argument("--item", action="append", type=parse_item, required=True, help="SKU[:QTY]")

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
        elif args.command == "checkout":
            order = place_order(store, args.customer, args.item)
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
