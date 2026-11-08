import { lazyLoadControllersFrom } from "@hotwired/stimulus-loading"
import { application } from "controllers/application"
import ScrollKeeper from "controllers/lc_scroll_keeper_controller"

// The sidebar's scroll keeper registers up front, so a reload restores its position before the first paint.
application.register("lc-scroll-keeper", ScrollKeeper)

lazyLoadControllersFrom("controllers", application)
