import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";
import { forwardRef, type ButtonHTMLAttributes } from "react";
import { cn } from "../lib/cn";
import { Spinner } from "./Spinner";

/**
 * A DISABLED FILLED BUTTON MUST NOT KEEP ITS FILL, and `disabled:opacity-50` alone is not enough.
 *
 * FOUND ON A LIVE STACK. A captain deployed a contract, the page correctly disabled the Deploy button,
 * and he reported "the deploy button is still blue it should have been disabled" - because a saturated
 * `bg-primary` at half opacity is still plainly blue, so the control went on looking pressable while
 * it was inert. A disabled control that looks live is indistinguishable from a page that has stopped
 * responding, and the reader's next move is to press it again: on that flow a second send is a second
 * `createIssuer` at the same contract number, or a `NoChange` revert, either of which costs gas.
 *
 * So the FILLED variants drop their fill when disabled and take a flat muted surface with a border,
 * which reads as unavailable at a glance and in a screenshot. Opacity is kept as a secondary hint and
 * is deliberately no longer the only one - it is invisible against a light background, which is exactly
 * how this shipped. `outline`, `ghost` and `link` are unchanged: they carry no fill to remove, so
 * opacity is already a real change of appearance for them.
 *
 * Pinned by `providerDeployButtonLifecycle.test.tsx`, which asserts the class list rather than a
 * screenshot so it is repeatable, and which asserts the opacity-only treatment is NOT the whole of it.
 *
 * THE INERT LOOK: no fill, a border, and the ordinary surface text colour.
 *
 * Applied UNPREFIXED and conditionally rather than as `disabled:bg-...`, and that is the load-bearing
 * part. `cn` is `twMerge`, which is last-wins for conflicting utilities - so appending this strips
 * `bg-primary` from the CLASS LIST itself rather than merely out-specifying it in the cascade. Two
 * things follow: the rendered class list is an honest description of what the button looks like (so a
 * test, a reviewer or a screenshot differ all read the same truth), and nothing rests on an argument
 * about Tailwind's variant ordering or on `:disabled` carrying more specificity than a plain class.
 *
 * **IT MUST NOT COVER `loading`, AND THAT EXCLUSION IS THE WHOLE OF WHY THERE ARE TWO FLAGS.** A loading
 * button is un-pressable (it carries the `disabled` attribute) but it is not UNAVAILABLE - it is busy
 * doing the thing you asked for, and a spinner says exactly that. Draining its fill would say the
 * opposite, and there are 73 `loading={…}` call sites across the portals, so getting this wrong is a
 * fleet-wide regression on every submit button that has ever spun. So the fill is removed for
 * `disabled && !loading`; `disabled` on the ELEMENT still comes from either.
 *
 * `hover:bg-surface-muted` is included so the class list mentions the fill NOWHERE. The hover state is
 * already unreachable (`disabled:pointer-events-none`), so this changes no pixel - it exists so that a
 * reader, a grep or a test looking for the fill finds none, rather than finding `hover:bg-primary/90`
 * and having to reason about whether it can fire.
 */
const INERT_FILL =
  "border border-border bg-surface-muted text-onSurface shadow-none hover:bg-surface-muted";

/** The variants that HAVE a fill to remove. `outline`, `ghost` and `link` carry none. */
const FILLED_VARIANTS = new Set(["primary", "secondary", "danger", "success"]);

export const buttonVariants = cva(
  "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:pointer-events-none disabled:opacity-50 [&_svg]:size-4 [&_svg]:shrink-0",
  {
    variants: {
      variant: {
        primary: "bg-primary text-onPrimary hover:bg-primary/90",
        secondary: "bg-surface-muted text-onSurface hover:bg-surface-muted/70",
        outline:
          "border border-border bg-surface text-onSurface hover:bg-surface-muted",
        ghost: "text-onSurface hover:bg-surface-muted",
        danger: "bg-danger text-onDanger hover:bg-danger/90",
        success: "bg-success text-onSuccess hover:bg-success/90",
        link: "text-primary underline-offset-4 hover:underline",
      },
      size: {
        sm: "h-8 px-3 text-xs",
        md: "h-10 px-4 py-2",
        lg: "h-11 px-6 text-base",
        icon: "h-10 w-10",
      },
    },
    defaultVariants: { variant: "primary", size: "md" },
  },
);

export interface ButtonProps
  extends ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {
  asChild?: boolean;
  loading?: boolean;
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, asChild = false, loading, children, disabled, ...props }, ref) => {
    const Comp = asChild ? Slot : "button";
    // TWO FLAGS, and they are not the same question. `unpressable` decides the ATTRIBUTE and is true
    // for either cause; `unavailable` decides the APPEARANCE and is false while loading, because a
    // busy button is not an unavailable one. See `INERT_FILL`.
    const unpressable = Boolean(disabled || loading);
    const unavailable = Boolean(disabled) && !loading;
    return (
      <Comp
        className={cn(
          buttonVariants({ variant, size }),
          // Before `className`, so a caller can still override deliberately, and after the variant so
          // `twMerge` removes the fill rather than layering on top of it.
          unavailable && FILLED_VARIANTS.has(variant ?? "primary") && INERT_FILL,
          className,
        )}
        ref={ref}
        disabled={unpressable}
        {...props}
      >
        {loading ? (
          <>
            <Spinner className="h-4 w-4" />
            {children}
          </>
        ) : (
          children
        )}
      </Comp>
    );
  },
);
Button.displayName = "Button";
