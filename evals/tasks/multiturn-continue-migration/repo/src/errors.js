/** An error with an HTTP status; the router turns it into a response. */
export class HttpError extends Error {
  constructor(status, message) {
    super(message);
    this.status = status;
  }
}
