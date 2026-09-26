import { createUser, deleteUser, getUser, listUsers } from "./handlers/users.js";
import { createTeam, deleteTeam, getTeam, listTeams } from "./handlers/teams.js";
import { createProject, deleteProject, getProject, listProjects } from "./handlers/projects.js";
import { createTask, deleteTask, getTask, listTasks } from "./handlers/tasks.js";
import { createComment, deleteComment, getComment, listComments } from "./handlers/comments.js";
import { createLabel, deleteLabel, getLabel, listLabels } from "./handlers/labels.js";
import { createInvoice, deleteInvoice, getInvoice, listInvoices } from "./handlers/invoices.js";
import { createPayment, deletePayment, getPayment, listPayments } from "./handlers/payments.js";
import { createCustomer, deleteCustomer, getCustomer, listCustomers } from "./handlers/customers.js";
import { createProduct, deleteProduct, getProduct, listProducts } from "./handlers/products.js";
import { createOrder, deleteOrder, getOrder, listOrders } from "./handlers/orders.js";
import { createShipment, deleteShipment, getShipment, listShipments } from "./handlers/shipments.js";
import { createWarehouse, deleteWarehouse, getWarehouse, listWarehouses } from "./handlers/warehouses.js";
import { createSupplier, deleteSupplier, getSupplier, listSuppliers } from "./handlers/suppliers.js";
import { createReport, deleteReport, getReport, listReports } from "./handlers/reports.js";
import { createWebhook, deleteWebhook, getWebhook, listWebhooks } from "./handlers/webhooks.js";
import { createSession, deleteSession, getSession, listSessions } from "./handlers/sessions.js";
import { createToken, deleteToken, getToken, listTokens } from "./handlers/tokens.js";
import { createNotification, deleteNotification, getNotification, listNotifications } from "./handlers/notifications.js";
import { createSubscription, deleteSubscription, getSubscription, listSubscriptions } from "./handlers/subscriptions.js";
import { createCoupon, deleteCoupon, getCoupon, listCoupons } from "./handlers/coupons.js";
import { createAudit, deleteAudit, getAudit, listAudits } from "./handlers/audits.js";
import { createExport, deleteExport, getExport, listExports } from "./handlers/exports.js";
import { getUsage, recordUsage } from "./handlers/usage.js";
import { getCatalog } from "./handlers/catalog.js";
import { errorResponse } from "./lib/http.js";
import { parseJson } from "./middleware/json.js";

export const ROUTES = [
  ["GET /users", listUsers],
  ["GET /users/:id", getUser],
  ["POST /users", createUser],
  ["DELETE /users/:id", deleteUser],
  ["GET /teams", listTeams],
  ["GET /teams/:id", getTeam],
  ["POST /teams", createTeam],
  ["DELETE /teams/:id", deleteTeam],
  ["GET /projects", listProjects],
  ["GET /projects/:id", getProject],
  ["POST /projects", createProject],
  ["DELETE /projects/:id", deleteProject],
  ["GET /tasks", listTasks],
  ["GET /tasks/:id", getTask],
  ["POST /tasks", createTask],
  ["DELETE /tasks/:id", deleteTask],
  ["GET /comments", listComments],
  ["GET /comments/:id", getComment],
  ["POST /comments", createComment],
  ["DELETE /comments/:id", deleteComment],
  ["GET /labels", listLabels],
  ["GET /labels/:id", getLabel],
  ["POST /labels", createLabel],
  ["DELETE /labels/:id", deleteLabel],
  ["GET /invoices", listInvoices],
  ["GET /invoices/:id", getInvoice],
  ["POST /invoices", createInvoice],
  ["DELETE /invoices/:id", deleteInvoice],
  ["GET /payments", listPayments],
  ["GET /payments/:id", getPayment],
  ["POST /payments", createPayment],
  ["DELETE /payments/:id", deletePayment],
  ["GET /customers", listCustomers],
  ["GET /customers/:id", getCustomer],
  ["POST /customers", createCustomer],
  ["DELETE /customers/:id", deleteCustomer],
  ["GET /products", listProducts],
  ["GET /products/:id", getProduct],
  ["POST /products", createProduct],
  ["DELETE /products/:id", deleteProduct],
  ["GET /orders", listOrders],
  ["GET /orders/:id", getOrder],
  ["POST /orders", createOrder],
  ["DELETE /orders/:id", deleteOrder],
  ["GET /shipments", listShipments],
  ["GET /shipments/:id", getShipment],
  ["POST /shipments", createShipment],
  ["DELETE /shipments/:id", deleteShipment],
  ["GET /warehouses", listWarehouses],
  ["GET /warehouses/:id", getWarehouse],
  ["POST /warehouses", createWarehouse],
  ["DELETE /warehouses/:id", deleteWarehouse],
  ["GET /suppliers", listSuppliers],
  ["GET /suppliers/:id", getSupplier],
  ["POST /suppliers", createSupplier],
  ["DELETE /suppliers/:id", deleteSupplier],
  ["GET /reports", listReports],
  ["GET /reports/:id", getReport],
  ["POST /reports", createReport],
  ["DELETE /reports/:id", deleteReport],
  ["GET /webhooks", listWebhooks],
  ["GET /webhooks/:id", getWebhook],
  ["POST /webhooks", createWebhook],
  ["DELETE /webhooks/:id", deleteWebhook],
  ["GET /sessions", listSessions],
  ["GET /sessions/:id", getSession],
  ["POST /sessions", createSession],
  ["DELETE /sessions/:id", deleteSession],
  ["GET /tokens", listTokens],
  ["GET /tokens/:id", getToken],
  ["POST /tokens", createToken],
  ["DELETE /tokens/:id", deleteToken],
  ["GET /notifications", listNotifications],
  ["GET /notifications/:id", getNotification],
  ["POST /notifications", createNotification],
  ["DELETE /notifications/:id", deleteNotification],
  ["GET /subscriptions", listSubscriptions],
  ["GET /subscriptions/:id", getSubscription],
  ["POST /subscriptions", createSubscription],
  ["DELETE /subscriptions/:id", deleteSubscription],
  ["GET /coupons", listCoupons],
  ["GET /coupons/:id", getCoupon],
  ["POST /coupons", createCoupon],
  ["DELETE /coupons/:id", deleteCoupon],
  ["GET /audits", listAudits],
  ["GET /audits/:id", getAudit],
  ["POST /audits", createAudit],
  ["DELETE /audits/:id", deleteAudit],
  ["GET /exports", listExports],
  ["GET /exports/:id", getExport],
  ["POST /exports", createExport],
  ["DELETE /exports/:id", deleteExport],
  ["GET /usage/:account", getUsage],
  ["POST /usage/:account", recordUsage],
  ["GET /catalog", getCatalog],
];

function match(pattern, method, path) {
  const [patternMethod, patternPath] = pattern.split(" ");
  if (patternMethod !== method) return null;
  const want = patternPath.split("/");
  const got = path.split("/");
  if (want.length !== got.length) return null;
  const params = {};
  for (let i = 0; i < want.length; i += 1) {
    if (want[i].startsWith(":")) params[want[i].slice(1)] = got[i];
    else if (want[i] !== got[i]) return null;
  }
  return params;
}

export function handle({ method, path, query = {}, body, headers = {} }) {
  for (const [pattern, handler] of ROUTES) {
    const params = match(pattern, method, path);
    if (params) return parseJson(handler)({ params, query, body, headers });
  }
  return errorResponse("NOT_FOUND", { path });
}
