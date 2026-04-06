import type { Meta, StoryObj } from "@storybook/react";
import { Modal, Button } from "@7d/ui";
import { useState } from "react";

const meta: Meta<typeof Modal> = {
  title: "Overlays/Modal",
  component: Modal,
  tags: ["autodocs"],
  parameters: { layout: "centered" },
};

export default meta;
type Story = StoryObj<typeof Modal>;

export const Default: Story = {
  render: () => {
    const [open, setOpen] = useState(false);
    return (
      <>
        <Button onClick={() => setOpen(true)}>Open modal</Button>
        <Modal
          open={open}
          onClose={() => setOpen(false)}
          title="Confirm action"
          description="Are you sure you want to proceed? This cannot be undone."
          footer={
            <>
              <Button variant="outline" onClick={() => setOpen(false)}>Cancel</Button>
              <Button variant="danger" onClick={() => setOpen(false)}>Delete</Button>
            </>
          }
        >
          <p className="text-sm text-text-secondary">
            All associated records will be permanently removed from the system.
          </p>
        </Modal>
      </>
    );
  },
};

export const Sizes: Story = {
  render: () => {
    const [size, setSize] = useState<"sm" | "md" | "lg" | "xl" | null>(null);
    return (
      <>
        <div className="flex gap-2">
          {(["sm", "md", "lg", "xl"] as const).map((s) => (
            <Button key={s} variant="outline" size="sm" onClick={() => setSize(s)}>
              {s.toUpperCase()}
            </Button>
          ))}
        </div>
        {size && (
          <Modal open onClose={() => setSize(null)} title={`Size: ${size}`} size={size}>
            <p className="text-sm text-text-secondary">Modal body content.</p>
          </Modal>
        )}
      </>
    );
  },
};
