import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@loom/ui-kit/components/ui/card";
import { SetupGuideVariantPanel } from "@loom/ui-kit/components/SetupGuideVariantPanel";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@loom/ui-kit/components/ui/tabs";
import type { SetupGuide } from "@loom/ui-kit/lib/api";

/**
 * Type-provided setup instructions with live values from the generated form.
 * Templates remain plain text: the connector supplies no HTML and the client
 * performs only the documented `{{fieldName}}` substitution.
 */
export function SetupGuidePanel({
  guide,
  typeId,
  formValues,
}: {
  guide: SetupGuide;
  typeId: string;
  formValues: Record<string, unknown>;
}) {
  const initialVariant = guide.variants[0];
  if (initialVariant === undefined) return null;

  return (
    <Card className="surface-panel h-fit">
      <CardHeader className="p-4 pb-0">
        <CardTitle className="text-base">Setup guide</CardTitle>
      </CardHeader>
      <CardContent className="p-4">
        <Tabs defaultValue={initialVariant.id}>
          <TabsList className="h-auto w-full justify-start overflow-x-auto">
            {guide.variants.map((variant) => (
              <TabsTrigger key={variant.id} value={variant.id} className="min-h-11 flex-1">
                {variant.label}
              </TabsTrigger>
            ))}
          </TabsList>
          {guide.variants.map((variant) => (
            <TabsContent
              key={variant.id}
              value={variant.id}
              forceMount
              className="data-[state=inactive]:hidden"
            >
              <SetupGuideVariantPanel
                variant={variant}
                typeId={typeId}
                formValues={formValues}
              />
            </TabsContent>
          ))}
        </Tabs>
      </CardContent>
    </Card>
  );
}
