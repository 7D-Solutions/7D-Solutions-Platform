export class ApiError extends Error {
  constructor(
    public readonly status: number,
    public readonly code: string,
    message: string,
  ) {
    super(message);
    this.name = "ApiError";
  }

  static fromResponse(status: number, body: unknown): ApiError {
    const b = body as Record<string, unknown> | null;
    if (b != null && typeof b.error === "string") {
      return new ApiError(status, String(status), b.error);
    }
    return new ApiError(status, String(status), `HTTP ${status}`);
  }
}
