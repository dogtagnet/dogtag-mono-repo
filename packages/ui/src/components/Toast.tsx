import * as ToastPrimitive from "@radix-ui/react-toast";
import { cva, type VariantProps } from "class-variance-authority";
import { X } from "lucide-react";
import {
  createContext,
  forwardRef,
  useCallback,
  useContext,
  useMemo,
  useState,
  type ComponentPropsWithoutRef,
  type ElementRef,
  type ReactNode,
} from "react";
import { cn } from "../lib/cn";

const toastVariants = cva(
  "pointer-events-auto relative flex w-full items-start justify-between gap-3 overflow-hidden rounded-md border p-4 shadow-lg data-[state=open]:animate-in data-[state=closed]:animate-out",
  {
    variants: {
      variant: {
        default: "border-border bg-surface text-onSurface",
        success: "border-success/40 bg-surface text-onSurface",
        danger: "border-danger/40 bg-surface text-onSurface",
      },
    },
    defaultVariants: { variant: "default" },
  },
);

export interface ToastItem {
  id: string;
  title?: string;
  description?: ReactNode;
  variant?: VariantProps<typeof toastVariants>["variant"];
  durationMs?: number;
}

interface ToastContextValue {
  toast: (t: Omit<ToastItem, "id">) => void;
  dismiss: (id: string) => void;
}

const ToastContext = createContext<ToastContextValue | null>(null);

const accent: Record<string, string> = {
  default: "bg-primary",
  success: "bg-success",
  danger: "bg-danger",
};

export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<ToastItem[]>([]);

  const dismiss = useCallback((id: string) => {
    setItems((prev) => prev.filter((t) => t.id !== id));
  }, []);

  const toast = useCallback((t: Omit<ToastItem, "id">) => {
    const id = Math.random().toString(36).slice(2);
    setItems((prev) => [...prev, { ...t, id }]);
  }, []);

  const value = useMemo(() => ({ toast, dismiss }), [toast, dismiss]);

  return (
    <ToastContext.Provider value={value}>
      <ToastPrimitive.Provider swipeDirection="right">
        {children}
        {items.map((t) => (
          <ToastPrimitive.Root
            key={t.id}
            duration={t.durationMs ?? 5000}
            onOpenChange={(open) => {
              if (!open) dismiss(t.id);
            }}
            className={cn(toastVariants({ variant: t.variant }))}
          >
            <span className={cn("absolute inset-y-0 left-0 w-1", accent[t.variant ?? "default"])} />
            <div className="flex flex-col gap-1 pl-2">
              {t.title && <ToastPrimitive.Title className="text-sm font-semibold">{t.title}</ToastPrimitive.Title>}
              {t.description && (
                <ToastPrimitive.Description className="text-sm text-muted">{t.description}</ToastPrimitive.Description>
              )}
            </div>
            <ToastPrimitive.Close className="rounded-sm text-muted opacity-70 hover:opacity-100">
              <X className="h-4 w-4" />
            </ToastPrimitive.Close>
          </ToastPrimitive.Root>
        ))}
        <ToastViewport />
      </ToastPrimitive.Provider>
    </ToastContext.Provider>
  );
}

const ToastViewport = forwardRef<
  ElementRef<typeof ToastPrimitive.Viewport>,
  ComponentPropsWithoutRef<typeof ToastPrimitive.Viewport>
>(({ className, ...props }, ref) => (
  <ToastPrimitive.Viewport
    ref={ref}
    className={cn(
      "fixed bottom-0 right-0 z-[100] flex max-h-screen w-full flex-col gap-2 p-4 sm:max-w-sm",
      className,
    )}
    {...props}
  />
));
ToastViewport.displayName = "ToastViewport";

export function useToast(): ToastContextValue {
  const ctx = useContext(ToastContext);
  if (!ctx) throw new Error("useToast must be used within a <ToastProvider>");
  return ctx;
}
