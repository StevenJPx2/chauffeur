from datetime import date

from .models import Order
from .pricing import PricingError, subtotal


def place_order(store, customer, lines, today=None):
    if not lines:
        raise PricingError("an order needs at least one line")
    today = today or date.today()
    sub = subtotal(store, lines)
    order = Order(store.next_order_id(), customer, list(lines), sub, sub, today)
    store.record_order(order)
    return order
