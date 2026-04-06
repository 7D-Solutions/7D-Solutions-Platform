import type { Meta, StoryObj } from "@storybook/react";
import { Drawer, Button } from "@7d/ui";
import { useState } from "react";

const meta: Meta<typeof Drawer> = {
  title: "Overlays/Drawer",
  component: Drawer,
  tags: ["autodocs"],
  parameters: { layout: "centered" },
};

export default meta;
type Story = StoryObj<typeof Drawer>;

export const Right: Story = {
  render: () => {
    const [open, setOpen] = useState(false);
    return (
      <>
        <Button onClick={() => setOpen(true)}>Open drawer</Button>
        <Drawer
          open={open}
          onClose={() => setOpen(false)}
          title="Settings"
          description="Adjust your preferences below."
          footer={
            <Button onClick={() => setOpen(false)}>Save changes</Button>
          }
        >
          <p className="text-sm text-text-secondary">Drawer body content goes here.</p>
        </Drawer>
      </>
    );
  },
};

export const Left: Story = {
  render: () => {
    const [open, setOpen] = useState(false);
    return (
      <>
        <Button onClick={() => setOpen(true)}>Open left drawer</Button>
        <Drawer
          open={open}
          onClose={() => setOpen(false)}
          title="Navigation"
          side="left"
        >
          <nav className="flex flex-col gap-1">
            {["Dashboard", "Inventory", "Orders", "Reports", "Settings"].map((item) => (
              <button
                key={item}
                className="text-left px-3 py-2 rounded-md text-sm hover:bg-bg-secondary"
                onClick={() => setOpen(false)}
              >
                {item}
              </button>
            ))}
          </nav>
        </Drawer>
      </>
    );
  },
};
