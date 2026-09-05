export class AppError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.name = 'AppError';
    this.code = code;
  }
}

export function assert(condition: unknown, code: string, message: string): asserts condition {
  if (condition) return;
  throw new AppError(code, message);
}
