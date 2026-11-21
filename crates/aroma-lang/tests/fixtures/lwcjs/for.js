import { LightningElement, api, track, wire } from 'lwc';
export default class Sample extends LightningElement {
sum(items) { let total = 0; for (let i = 0; i < items.length; i++) { total += items[i]; } return total; }
}
