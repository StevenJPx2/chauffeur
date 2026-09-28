from datetime import date

from .models import Order
from .pricing import PricingError, check_code, discount_amount, money, subtotal


def place_order(store, customer, lines, code=None, today=None):
    if not lines:
        raise PricingError("an order needs at least one line")
    today = today or date.today()
    sub = subtotal(store, lines)
    off = money(0)
    applied = None
    if code:
        discount = check_code(store, code, customer, today)
        off = discount_amount(discount, sub)
        applied = discount.code
    order = Order(store.next_order_id(), customer, list(lines), sub, money(sub - off), today, off, applied)
    store.record_order(order)
    return order
