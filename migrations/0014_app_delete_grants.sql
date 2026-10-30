-- The service role (acom_app) gets SELECT, INSERT and UPDATE on every table by default
-- (infra/postgres/init.sql). DELETE only where removing a row is part of the design:
--   sales.cart_line                remove_cart_item, clear_cart
--   customer.preference            memory undo
--   customer.postal_address        removing a saved address
--   customer.wishlist_item         remove_from_wishlist
--   promotion.unit_price_specification, promotion.coupon,
--   merchandising.display_booking, merchandising.banner
--                                  undoing a change set before its sale, coupon or booking starts
--   platform.change_set_item       items taken out of a draft change set
--   platform.idempotency_key       a failed first attempt is cleared so a retry can run
--   platform.session               expired sessions are cleaned up
-- Everything else is kept: state changes are updates, and the action log is append-only.

GRANT DELETE ON
  sales.cart_line,
  customer.preference,
  customer.postal_address,
  customer.wishlist_item,
  promotion.unit_price_specification,
  promotion.coupon,
  merchandising.display_booking,
  merchandising.banner,
  platform.change_set_item,
  platform.idempotency_key,
  platform.session
TO acom_app;
