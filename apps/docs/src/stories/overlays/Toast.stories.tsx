import type { Meta, StoryObj } from "@storybook/react";
import { ToastContainer, Button } from "@7d/ui";
import type { ToastProps } from "@7d/ui";
import { useState } from "react";

const meta: Meta = {
  title: "Overlays/Toast",
  tags: ["autodocs"],
  parameters: { layout: "centered" },
};

export default meta;
type Story = StoryObj;

let nextId = 1;

export const Default: Story = {
  render: () => {
    const [toasts, setToasts] = useState<ToastProps[]>([]);

    const add = (variant: ToastProps["variant"]) => {
      const id = String(nextId++);
      setToasts((prev) => [
        ...prev,
        {
          id,
          message: `${variant ?? "default"} notification`,
          variant,
          duration: 4000,
          onDismiss: (dismissId) =>
            setToasts((prev) => prev.filter((t) => t.id !== dismissId)),
        },
      ]);
    };

    return (
      <>
        <div className="flex flex-wrap gap-2">
          <Button size="sm" onClick={() => add("default")}>Default</Button>
          <Button size="sm" variant="secondary" onClick={() => add("success")}>Success</Button>
          <Button size="sm" variant="outline" onClick={() => add("info")}>Info</Button>
          <Button size="sm" variant="danger" onClick={() => add("danger")}>Danger</Button>
          <Button size="sm" onClick={() => add("warning")}>Warning</Button>
        </div>
        <ToastContainer toasts={toasts} position="bottom-right" />
      </>
    );
  },
};
