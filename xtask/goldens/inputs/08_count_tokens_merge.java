import java.util.Map;
import java.util.TreeMap;

class CountTokensMerge {
    Map<String, Integer> countTokens(String text) {
        Map<String, Integer> counts = new TreeMap<>();
        String[] tokens = text.toLowerCase().split(",");
        for (String token : tokens) {
            counts.merge(token.trim(), 1, Integer::sum);
        }
        counts.remove("");
        int distinct = counts.size();
        System.out.println(distinct);
        return counts;
    }
}
