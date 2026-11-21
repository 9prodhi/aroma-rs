import java.util.ArrayList;
import java.util.List;

class ParseDoubleList {
    List<Double> parseAll(String[] parts) {
        if (parts == null) {
            return new ArrayList<>();
        }
        List<Double> numbers = new ArrayList<>();
        int failures = 0;
        for (String part : parts) {
            try {
                numbers.add(Double.parseDouble(part.trim()));
            } catch (NumberFormatException e) {
                failures++;
            }
        }
        System.out.println(failures);
        return numbers;
    }
}
