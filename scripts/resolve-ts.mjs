// Lets the check scripts import viewer sources that use extensionless specifiers.
//
// `moduleResolution: "bundler"` lets `./sourcemap` mean `./sourcemap.ts`, which every
// bundler resolves and Node does not — so a module with runtime relative imports could
// not be driven headlessly at all. The modules that could (`keys`, `sourcemap`,
// `patch`) were the ones whose imports happen to be type-only, which is a poor reason
// for a module to be testable.
//
// Used as `node --import ./scripts/resolve-ts.mjs …`.

import { register } from "node:module";

register(
  "data:text/javascript," +
    encodeURIComponent(`
      export async function resolve(specifier, context, next) {
        if (specifier.startsWith(".") && !/\\.\\w+$/.test(specifier)) {
          try {
            return await next(specifier + ".ts", context);
          } catch {
            // not a TypeScript module — fall through to the default resolution
          }
        }
        return next(specifier, context);
      }
    `),
);
